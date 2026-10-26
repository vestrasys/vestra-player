// scripts/check-builtin-versions.mjs: a built-in plugin whose shipped files
// changed must carry a higher version (the host upgrades seeded built-ins
// only to a newer version).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  check,
  isHigher,
  isShippedFile,
  listedOf,
  parseVersion,
  shippedChanges,
  verdict,
} from "../../scripts/check-builtin-versions.mjs";

const nothingListed = () => false;
const plugin = (manifest, packageJson = {}, files = []) => ({
  manifest,
  packageJson,
  present: (rel) => files.includes(rel),
});
const manifest = (version, extra = {}) => ({
  id: "vestra.demo",
  version,
  entry: "main.ts",
  ...extra,
});

test("versions compare as the host reads them", () => {
  assert.deepEqual(parseVersion("1.2.3"), [1, 2, 3]);
  for (const bad of ["1.2", "1.2.3.4", "01.2.3", "1.2.3-beta", "", null])
    assert.equal(parseVersion(bad), null, String(bad));
  assert.ok(isHigher("0.1.10", "0.1.9"));
  assert.ok(isHigher("1.0.0", "0.9.9"));
  assert.ok(!isHigher("0.1.4", "0.1.4"));
  assert.ok(!isHigher("0.1.3", "0.1.4"));
  assert.ok(!isHigher("bad", "0.1.4"));
  assert.ok(!isHigher(undefined, "0.1.4"));
});

test("shipped files follow the SDK's build", () => {
  const shipped = (rel, listed = nothingListed) => isShippedFile(rel, listed);
  for (const rel of [
    "main.ts",
    "ui/panel.slint",
    "worker/run.js",
    "locales/de.po",
    "dependency-inventory.json",
  ])
    assert.ok(shipped(rel), rel);
  for (const rel of [
    "tests/main.test.ts",
    "tests/fixtures/a.json",
    "vendor/lib.js",
    "dist/main.js",
    "node_modules/x/index.js",
    "ui/node_modules/x.js",
    "evidence/shot.png",
    ".gitignore",
    ".build/x.js",
    "tsconfig.json",
    "tsconfig.test.json",
    "types.d.ts",
    "lib/types.d.mts",
    "locales/messages.pot",
    "notes/de.po",
    "locales/old/de.po",
    "de.po",
    "README.md",
    "notes.md",
    "debug.log",
    "old.vxp",
    "manifest.json",
    "package.json",
    "package-lock.json",
  ])
    assert.ok(!shipped(rel), rel);
  // Listed paths ship whatever their name: assets and display files.
  const listed = listedOf(
    { icon: "assets/icon.png" },
    { vestra: { assets: ["vendor", "licenses"] } },
    (rel) => rel === "README.md",
  );
  for (const rel of [
    "vendor/lib.js",
    "vendor/@scope/pkg/dist/a.d.ts",
    "vendor/@scope/pkg/README.md",
    "licenses/MIT.txt",
    "assets/icon.png",
    "README.md",
  ])
    assert.ok(shipped(rel, listed), rel);
  assert.ok(
    !shipped("vendored/x.d.ts", listed),
    "prefix is a path, not a string",
  );
  assert.ok(!shipped("CHANGELOG.md", listed), "absent changelog");
  // A manifest that names its readme replaces the default README.md.
  const named = listedOf({ readme: "docs/guide.md" }, {}, () => true);
  assert.ok(shipped("docs/guide.md", named));
  assert.ok(!shipped("README.md", named));
  assert.ok(shipped("CHANGELOG.md", named));
});

test("manifest and package.json count only for what pack ships", () => {
  const base = plugin(manifest("1.0.0"));
  assert.deepEqual(shippedChanges([], base, plugin(manifest("1.0.1"))), []);
  assert.deepEqual(
    shippedChanges(
      ["manifest.json"],
      base,
      plugin(manifest("1.0.0", { name: "New" })),
    ),
    ["manifest.json (other than version)"],
  );
  // devDependencies, scripts and package.json's own version are not shipped.
  const pkg = {
    version: "1.0.0",
    devDependencies: { a: "1" },
    vestra: { assets: ["vendor"] },
  };
  const withPkg = plugin(manifest("1.0.0"), pkg);
  assert.deepEqual(
    shippedChanges(
      ["package.json"],
      withPkg,
      plugin(manifest("1.0.0"), {
        ...pkg,
        version: "2.0.0",
        devDependencies: { a: "2" },
      }),
    ),
    [],
  );
  assert.deepEqual(
    shippedChanges(
      ["package.json"],
      withPkg,
      plugin(manifest("1.0.0"), {
        ...pkg,
        vestra: { assets: ["vendor", "worker"] },
      }),
    ),
    ["package.json (vestra options or bundled dependencies)"],
  );
  // A bundled plugin ships its dependencies, pinned by the lock file.
  const bundled = {
    dependencies: { jsnes: "2.1.0" },
    vestra: { bundle: true },
  };
  assert.deepEqual(
    shippedChanges(
      ["package.json"],
      plugin(manifest("1.0.0"), bundled),
      plugin(manifest("1.0.0"), {
        ...bundled,
        dependencies: { jsnes: "2.2.0" },
      }),
    ),
    ["package.json (vestra options or bundled dependencies)"],
  );
  assert.deepEqual(
    shippedChanges(
      ["package-lock.json"],
      plugin(manifest("1.0.0"), bundled),
      plugin(manifest("1.0.0"), bundled),
    ),
    ["package-lock.json (bundled plugin)"],
  );
  assert.deepEqual(shippedChanges(["package-lock.json"], withPkg, withPkg), []);
  // A listed path on either side counts (an asset dropped at head).
  assert.deepEqual(
    shippedChanges(
      ["vendor/lib.js"],
      plugin(manifest("1.0.0"), pkg),
      plugin(manifest("1.0.0"), { ...pkg, vestra: {} }),
    ),
    ["package.json (vestra options or bundled dependencies)", "vendor/lib.js"],
  );
});

test("a changed built-in needs a higher version; a new one needs nothing", () => {
  const base = plugin(manifest("1.0.0"));
  assert.equal(
    verdict("demo", ["tests/a.test.ts"], base, plugin(manifest("1.0.0"))),
    null,
  );
  assert.equal(
    verdict("demo", ["main.ts"], base, plugin(manifest("1.0.1"))),
    null,
  );
  assert.equal(
    verdict("demo", ["main.ts"], plugin(null), plugin(manifest("0.1.0"))),
    null,
  );
  const same = verdict(
    "demo",
    ["main.ts", "ui/a.slint"],
    base,
    plugin(manifest("1.0.0")),
  );
  assert.match(
    same,
    /^demo: shipped files changed but version 1\.0\.0 is not higher than 1\.0\.0/,
  );
  assert.match(same, /main\.ts/);
  assert.match(same, /ui\/a\.slint/);
  assert.match(
    verdict("demo", ["main.ts"], base, plugin(manifest("0.9.0"))),
    /0\.9\.0 is not higher than 1\.0\.0/,
  );
});

test("the check compares a release tag with a commit or the working tree", (t) => {
  const repo = mkdtempSync(join(tmpdir(), "vestra-builtin-versions-"));
  t.after(() => rmSync(repo, { recursive: true, force: true }));
  const git = (...args) => {
    const result = spawnSync(
      "git",
      [
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "tag.gpgsign=false",
        ...args,
      ],
      { cwd: repo, encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim();
  };
  const write = (rel, text) => {
    mkdirSync(join(repo, rel, ".."), { recursive: true });
    writeFileSync(join(repo, rel), text);
  };
  const demo = "src/plugins/demo";
  git("init", "-q");
  write(`${demo}/manifest.json`, JSON.stringify(manifest("1.0.0")));
  write(`${demo}/main.ts`, "export {};\n");
  write(`${demo}/tests/main.test.ts`, "\n");
  git("add", "-A");
  git("commit", "-q", "-m", "release");
  git("tag", "v1.0.0");

  write(`${demo}/tests/main.test.ts`, "// more tests\n");
  write(
    "src/plugins/fresh/manifest.json",
    JSON.stringify(manifest("0.1.0", { id: "vestra.fresh" })),
  );
  write("src/plugins/fresh/main.ts", "export {};\n");
  assert.deepEqual(
    check(repo).problems,
    [],
    "tests and a new plugin need no bump",
  );

  write(`${demo}/main.ts`, "export const changed = 1;\n");
  const tree = check(repo);
  assert.equal(tree.base, "v1.0.0");
  assert.equal(tree.problems.length, 1);
  assert.match(tree.problems[0], /^demo:/);
  git("add", "-A");
  git("commit", "-q", "-m", "change");
  const unbumped = git("rev-parse", "HEAD");

  write(`${demo}/manifest.json`, JSON.stringify(manifest("1.0.1")));
  assert.deepEqual(check(repo).problems, []);
  git("add", "-A");
  git("commit", "-q", "-m", "bump");
  assert.equal(
    check(repo, { base: "v1.0.0", head: unbumped }).problems.length,
    1,
  );
  assert.deepEqual(check(repo, { base: "v1.0.0", head: "HEAD" }).problems, []);
  // Against the base branch: the bump commit alone changes no shipped file.
  assert.deepEqual(check(repo, { base: unbumped, head: "HEAD" }).problems, []);
  // A missing base (a shallow clone without the tag) says what to fetch.
  assert.throws(
    () => check(repo, { base: "v9.9.9" }),
    /base ref v9\.9\.9 not found; fetch tags and full history \(fetch-depth: 0\)/,
  );
});
