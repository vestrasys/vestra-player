// The reference the agent reads and the toolchain its tools run: both are
// the player's own SDK (vendored by tests/vendor-sdk.mjs), and the agent can
// read but never change them.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { Toolchain } from "../toolchain.ts";
import { WorkFolder } from "../worker/files.ts";
import { Folders } from "../worker/tools.ts";
import { dataDirectory, server, startAi } from "./fixture.mjs";
import { differences, sources } from "./vendor-sdk.mjs";

const plugin = path.resolve(import.meta.dirname, "..");

test("vendor/ and knowledge/ match a fresh build from the player's SDK (run `npm run vendor:sdk` after an SDK upgrade)", async () => {
  assert.deepEqual(await differences(), []);
  const installed = JSON.parse(await fs.readFile(path.join(sources().sdk, "package.json"), "utf8")).version;
  assert.equal(new Toolchain(plugin, "/unused").sdkVersion, installed);
});

test("the reference: an index with the modules and capabilities, the full text with the examples", async () => {
  const toolchain = new Toolchain(plugin, "/unused");
  const index = await fs.readFile(toolchain.docsIndex(), "utf8");
  const full = await fs.readFile(path.join(path.dirname(toolchain.docsIndex()), "llms-full.txt"), "utf8");
  assert.ok(index.length < 64 * 1024, `llms.txt is ${index.length} bytes`);
  for (const word of ["plugin.commands", "plugin.windows", "ui.window", "dist/plugin/types.d.ts"]) assert.ok(index.includes(word), word);
  for (const example of ["hello", "blur", "signal-generator", "bookmark", "panel-menu", "menus", "theme-widgets", "markdown"]) assert.ok(full.includes(`examples/${example}`), example);
  assert.ok(toolchain.docsRoots().every((root) => root.startsWith(plugin)));
});

test("the agent's tools read the reference but write only the work folder", async () => {
  const root = await dataDirectory();
  await fs.mkdir(path.join(root, "work"));
  const toolchain = new Toolchain(plugin, root);
  const docs = await Promise.all(toolchain.docsRoots().map((r) => WorkFolder.open(r)));
  const folders = new Folders(await WorkFolder.open(path.join(root, "work")), docs);
  const index = await fs.realpath(toolchain.docsIndex());
  assert.notEqual(folders.of(index), folders.work);
  assert.match((await folders.of(index).readFile(index)).toString(), /SDK/);
  assert.equal(folders.of("notes.md"), folders.work);
  // write and edit go to the work folder only, which refuses the path.
  await assert.rejects(folders.work.writeFile(index, "x"), /工作目录以外/);
});

test("a write into the reference fails and the file stays as it is", async () => {
  const toolchain = new Toolchain(plugin, "/unused");
  const index = await fs.realpath(toolchain.docsIndex());
  const before = await fs.readFile(index, "utf8");
  const mock = await server((req) =>
    req.body.messages.at(-1).role === "tool" ? { text: ["不能写。"] } : { toolCalls: [{ name: "write", args: { path: index, content: "changed" } }] },
  );
  const ai = await startAi({ compat: mock.url });
  await ai.ask("改一下参考资料");
  assert.equal(ai.rows()[1].detail, `✗ 写入 参考资料/${path.basename(path.dirname(index))}/llms.txt`);
  assert.equal(await fs.readFile(index, "utf8"), before);
  await ai.stop();
});
