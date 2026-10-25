// The reference the agent reads and the toolchain its tools run: both are
// the player's own SDK (vendored by tests/vendor-sdk.mjs), and the agent can
// read but never change them.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { Toolchain } from "../toolchain.ts";
import { WorkFolder } from "../worker/files.ts";
import { Folders, createTools } from "../worker/tools.ts";
import { dataDirectory, server, startAi } from "./fixture.mjs";
import { differences, sources } from "./vendor-sdk.mjs";

const plugin = path.resolve(import.meta.dirname, "..");

test("vendor/ matches a fresh build from the player's SDK (run `npm run vendor:sdk` after an SDK upgrade)", async () => {
  assert.deepEqual(await differences(), []);
  const installed = JSON.parse(await fs.readFile(path.join(sources().sdk, "package.json"), "utf8")).version;
  assert.equal(new Toolchain(plugin, "/unused").sdkVersion, installed);
});

test("the reference is the SDK's own llms.txt: an index with the modules and capabilities, the full text with the examples", async () => {
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
  await assert.rejects(folders.work.writeFile(index, "x"), /outside the working folder/);
});

test("the reference by its short name: sdk/… reads, lists and greps the SDK; writes there are refused", async () => {
  const root = await dataDirectory();
  await fs.mkdir(path.join(root, "work"));
  const toolchain = new Toolchain(plugin, root);
  const docs = await Promise.all(toolchain.docsRoots().map((r) => WorkFolder.open(r)));
  const folders = new Folders(await WorkFolder.open(path.join(root, "work")), docs);
  const tools = Object.fromEntries(createTools(folders).map((t) => [t.name, t]));
  const run = async (name, args) => (await tools[name].execute("1", args, undefined, undefined, undefined)).content.map((c) => c.text).join("");
  const index = await fs.readFile(toolchain.docsIndex(), "utf8");
  assert.equal(await run("read", { path: "sdk/llms.txt", limit: 3 }).then((t) => t.split("\n")[0]), index.split("\n")[0]);
  assert.equal(await run("read", { path: "@sdk/llms.txt", limit: 1 }).then((t) => t.split("\n")[0]), index.split("\n")[0]);
  assert.match(await run("ls", { path: "sdk/dist/plugin" }), /types\.d\.ts/);
  const grep = await run("grep", { pattern: "interface TestHost|class TestHost", path: "sdk/dist/plugin/testing.d.ts", limit: 1 });
  assert.match(grep, /^sdk\/dist\/plugin\/testing\.d\.ts:\d+: /);
  await assert.rejects(run("write", { path: "sdk/llms.txt", content: "x" }), /read-only/);
  await assert.rejects(run("write", { path: "sdk/new.txt", content: "x" }), /read-only/);
  await assert.rejects(run("delete", { path: "sdk/llms.txt" }), /read-only/);
  assert.equal(await fs.readFile(toolchain.docsIndex(), "utf8"), index);
  await assert.rejects(fs.access(path.join(root, "work", "sdk")));
});

test("a write into the reference fails and the file stays as it is", async () => {
  const toolchain = new Toolchain(plugin, "/unused");
  const index = await fs.realpath(toolchain.docsIndex());
  const before = await fs.readFile(index, "utf8");
  const mock = await server((req) => {
    const last = req.body.messages.at(-1);
    if (last.role === "tool") return { text: ["不能写。"] };
    const content = typeof last.content === "string" ? last.content : last.content.map((p) => p.text).join("");
    return { toolCalls: [{ name: "write", args: { path: content.includes("短") ? "sdk/llms.txt" : index, content: "changed" } }] };
  });
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("改一下参考资料");
  assert.equal(await ai.calls(1), `✗ 写入 参考资料/${path.basename(path.dirname(index))}/llms.txt`);
  await ai.ask("用短路径改一下参考资料");
  assert.equal(await ai.calls(-2), "✗ 写入 参考资料/sdk/llms.txt");
  assert.equal(await fs.readFile(index, "utf8"), before);
  await ai.stop();
});
