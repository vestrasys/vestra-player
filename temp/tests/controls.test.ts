// 实验挂载点回归：不是最终布局 API；所有插件按 manifest 挂载，不识别插件名。
import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TestHost } from "./support/host.ts";

test("controls is a generic unique slot, independently removable from ordinary panels", async (t) => {
  const host = new TestHost();
  t.after(() => host.close());
  const directory = await mkdtemp(join(tmpdir(), "vestra-controls-"));
  t.after(() => rm(directory, { force: true, recursive: true }));
  async function fixture(id: string, region?: string) {
    const root = join(directory, id);
    await mkdir(root);
    await writeFile(
      join(root, "index.ts"),
      "// 实验 fixture。\nexport async function activate() {}\n",
    );
    await writeFile(
      join(root, "ui.slint"),
      `// 实验 fixture。\nexport global ${id}State { callback action(string,float); }\nexport component ${id}Panel inherits Rectangle {}\n`,
    );
    const path = join(root, "manifest.json");
    await writeFile(
      path,
      JSON.stringify({
        id,
        name: id,
        experimental: true,
        entry: "index.ts",
        ui: {
          file: "ui.slint",
          component: `${id}Panel`,
          global: `${id}State`,
          ...(region ? { region } : {}),
        },
      }),
    );
    return path;
  }
  assert.ok(
    (await host.request<string[]>("capabilities", {})).includes("ui.controls"),
  );
  const first = await fixture("FirstControl", "controls");
  const second = await fixture("SecondControl", "controls");
  const panel = await fixture("OrdinaryTool");
  const invalid = await fixture("InvalidRegion", "bottom");
  await host.load(first);
  await host.load(panel);
  await assert.rejects(host.load(second), /one controls/i);
  await assert.rejects(host.load(invalid), /region/i);
  assert.equal((await host.inspect()).plugins.length, 2);
  await host.unload("FirstControl");
  await host.load(second);
  assert.deepEqual((await host.inspect()).plugins.map((p) => p.id).sort(), [
    "OrdinaryTool",
    "SecondControl",
  ]);
});
