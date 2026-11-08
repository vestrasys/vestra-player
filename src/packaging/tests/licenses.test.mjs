// node --test packaging/tests/ (npm run test:packaging)
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import {
  checkPluginPackages,
  loadThirdParty,
  packedByBuild,
  pluginDirectory,
  pluginPackages,
  sourcePlugin,
} from "../check-licenses.mjs";
import { source } from "../lib.mjs";

const config = await loadThirdParty();
const allowed = new Set(config.policy.allowed);

/** An in-memory plugin: { path: text }. */
const memory = (tree) => ({
  files: Object.keys(tree).sort(),
  read: async (rel) => (rel in tree ? Buffer.from(tree[rel]) : undefined),
});
const json = (value) => JSON.stringify(value);
const components = [
  {
    name: "bundle",
    vxp: "x",
    inventory: "dependency-inventory.json",
    inventory_licenses: "licenses",
  },
  { name: "vendored", vxp: "x", vendor: "vendor" },
  { name: "logo", vxp: "x", vxp_license_files: ["licenses/logo/NOTICE.md"] },
];
const good = {
  "dependency-inventory.json": json([
    {
      name: "@a/b",
      version: "1.0.0",
      license: "MIT",
      licenseFiles: ["LICENSE"],
    },
    {
      name: "c",
      version: "2.0.0",
      license: "(MIT OR GPL-2.0)",
      licenseFiles: ["license.md"],
    },
  ]),
  "licenses/@a__b/LICENSE": "MIT",
  "licenses/@a__b/package.json": "{}",
  "licenses/c/license.md": "MIT",
  "licenses/logo/NOTICE.md": "notice",
  "vendor/bundle/package.json": json({ name: "@a/b", version: "1.0.0" }),
  "vendor/bundle/b.mjs": "",
  "vendor/@s/t/package.json": json({
    name: "@s/t",
    version: "3.0.0",
    license: "Apache-2.0",
  }),
  "vendor/@s/t/LICENSE.txt": "Apache",
  "vendor/@s/t/ThirdPartyNoticeText.txt": "notices",
  "vendor/@s/t/lib/deep/x.d.ts": "",
  "vendor/u/package.json": json({
    name: "u",
    version: "4.0.0",
    license: "MIT",
  }),
  "vendor/u/LICENCE": "MIT",
};
const check = (tree) =>
  checkPluginPackages(components, memory(tree), allowed, "p");

test("a plugin whose bundled and vendored packages all have licenses passes", async () => {
  assert.deepEqual(await check(good), []);
});

test("a version bump in the inventory or vendor/ needs no third-party.toml edit", async () => {
  const tree = { ...good };
  tree["dependency-inventory.json"] = good["dependency-inventory.json"].replace(
    "2.0.0",
    "2.1.0",
  );
  tree["vendor/@s/t/package.json"] = json({
    name: "@s/t",
    version: "3.1.0",
    license: "Apache-2.0",
  });
  tree["vendor/w/package.json"] = json({
    name: "w",
    version: "1.0.0",
    license: "ISC",
  });
  tree["vendor/w/LICENSE"] = "ISC";
  assert.deepEqual(await check(tree), []);
});

test("missing license files, disallowed licenses and strays are reported", async () => {
  const tree = { ...good };
  delete tree["licenses/c/license.md"]; // e.g. dropped by the packer
  tree["licenses/stale/LICENSE"] = "";
  tree["vendor/loose.js"] = "";
  tree["vendor/u/package.json"] = json({
    name: "u",
    version: "4.0.0",
    license: "SSPL-1.0",
  });
  delete tree["vendor/@s/t/LICENSE.txt"];
  tree["vendor/v/package.json"] = json({ name: "not-v", license: "MIT" });
  tree["vendor/v/LICENSE"] = "";
  assert.deepEqual((await check(tree)).sort(), [
    "p: @s/t@3.0.0: no license file",
    "p: c@2.0.0: missing licenses/c/license.md",
    "p: licenses/stale/LICENSE is in no component of third-party.toml",
    "p: u@4.0.0: SSPL-1.0 is not allowed",
    "p: vendored: vendor/loose.js belongs to no vendored package",
    "p: vendored: vendor/v/package.json names not-v, not v",
  ]);
});

test("an inventory entry without a license is reported", async () => {
  const tree = { ...good };
  tree["dependency-inventory.json"] = json([
    {
      name: "@a/b",
      version: "1.0.0",
      license: null,
      licenseFiles: ["LICENSE"],
    },
    {
      name: "c",
      version: "2.0.0",
      license: "MIT",
      licenseFiles: ["license.md"],
    },
  ]);
  assert.deepEqual(await check(tree), ["p: @a/b@1.0.0: no license"]);
});

test("every plugin source described by inventory / vendor components passes", async () => {
  const ids = new Set(
    config.component.filter((c) => c.inventory || c.vendor).map((c) => c.vxp),
  );
  assert.ok(ids.size);
  for (const id of ids) {
    const dir = await pluginDirectory(id);
    assert.ok(dir, id);
    const plugin = await sourcePlugin(join(source, "plugins", dir));
    const list = config.component.filter((c) => c.vxp === id);
    assert.deepEqual(await checkPluginPackages(list, plugin, allowed, dir), []);
  }
});

test("a vendored package records its license and the notices beside it", async () => {
  const { lists } = await pluginPackages(components, memory(good));
  const vendored = lists.find((list) => list.component.vendor).packages;
  assert.deepEqual(
    vendored.map((pkg) => [pkg.name, pkg.files]),
    [
      [
        "@s/t",
        ["vendor/@s/t/LICENSE.txt", "vendor/@s/t/ThirdPartyNoticeText.txt"],
      ],
      ["u", ["vendor/u/LICENCE"]],
    ],
  );
});

test("the workshop's vendored SDK and TypeScript notices are covered", async () => {
  const dir = await pluginDirectory("vestra.workshop");
  const { lists } = await pluginPackages(
    config.component.filter((c) => c.vxp === "vestra.workshop"),
    await sourcePlugin(join(source, "plugins", dir)),
  );
  const vendored = new Map(
    lists
      .find((list) => list.component.vendor)
      .packages.map((p) => [p.name, p]),
  );
  assert.deepEqual(vendored.get("@vestra-player/sdk").files, [
    "vendor/@vestra-player/sdk/LICENSE",
  ]);
  assert.deepEqual(vendored.get("typescript").files, [
    "vendor/typescript/LICENSE.txt",
    "vendor/typescript/ThirdPartyNoticeText.txt",
  ]);
});

test("a license file reaches the package only when the build copies it", () => {
  // Markdown and translation sources are skipped unless listed in assets.
  assert.equal(packedByBuild("licenses/still/NOTICE.md"), false);
  assert.equal(packedByBuild("licenses/still/NOTICE.md", ["licenses"]), true);
  assert.equal(packedByBuild("licenses/jsnes/LICENSE"), true);
  assert.equal(packedByBuild("assets/still.jpg"), true);
  assert.equal(packedByBuild("tests/LICENSE"), false);
  assert.equal(packedByBuild("vendor/x/LICENSE", ["vendor"]), true);
  assert.equal(packedByBuild(".build/LICENSE"), false);
  assert.equal(packedByBuild("licenses/x/LICENSE.md", ["licenses/y"]), false);
});

test("every vxp_license_files entry is copied by its plugin build", async () => {
  for (const component of config.component.filter((c) => c.vxp)) {
    const dir = await pluginDirectory(component.vxp);
    assert.ok(dir, component.vxp);
    const file = join(source, "plugins", dir, "package.json");
    const assets = existsSync(file)
      ? (JSON.parse(await readFile(file, "utf8")).vestra?.assets ?? [])
      : [];
    for (const rel of component.vxp_license_files ?? [])
      assert.ok(packedByBuild(rel, assets), `${component.name}: ${rel}`);
  }
});
