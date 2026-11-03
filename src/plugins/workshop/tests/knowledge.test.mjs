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

test("the reference is the SDK's own documentation tree, vendored with the package (no llms-full.txt)", async () => {
  const toolchain = new Toolchain(plugin, "/unused");
  for (const file of ["dist/docs.js", "dist/docs-tree.json.gz", "dist/plugin/types.d.ts", "dist/plugin/testing.d.ts", "llms.txt"]) await fs.access(path.join(toolchain.sdk, file));
  await assert.rejects(fs.access(path.join(toolchain.sdk, "llms-full.txt")));
  assert.ok(toolchain.docsRoots().every((root) => root.startsWith(plugin)));
});

test("the agent's tools read the reference but write only the work folder", async () => {
  const root = await dataDirectory();
  await fs.mkdir(path.join(root, "work"));
  const toolchain = new Toolchain(plugin, root);
  const docs = await Promise.all(toolchain.docsRoots().map((r) => WorkFolder.open(r)));
  const folders = new Folders(await WorkFolder.open(path.join(root, "work")), docs);
  const index = await fs.realpath(path.join(toolchain.sdk, "llms.txt"));
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
  const folders = await Folders.open(path.join(root, "work"), toolchain.docsRoots());
  const tools = Object.fromEntries(createTools(folders).map((t) => [t.name, t]));
  const run = async (name, args) => (await tools[name].execute("1", args, undefined, undefined, undefined)).content.map((c) => c.text).join("");
  const index = await fs.readFile(path.join(toolchain.sdk, "llms.txt"), "utf8");
  assert.equal(await run("read", { path: "sdk/llms.txt", limit: 3 }).then((t) => t.split("\n")[0]), index.split("\n")[0]);
  assert.equal(await run("read", { path: "@sdk/llms.txt", limit: 1 }).then((t) => t.split("\n")[0]), index.split("\n")[0]);
  assert.match(await run("ls", { path: "sdk/dist/plugin" }), /types\.d\.ts/);
  const grep = await run("grep", { pattern: "interface TestHost|class TestHost", path: "sdk/dist/plugin/testing.d.ts", limit: 1 });
  assert.match(grep, /^sdk\/dist\/plugin\/testing\.d\.ts:\d+: /);
  await assert.rejects(run("write", { path: "sdk/llms.txt", content: "x" }), /read-only/);
  await assert.rejects(run("write", { path: "sdk/new.txt", content: "x" }), /read-only/);
  await assert.rejects(run("delete", { path: "sdk/llms.txt" }), /read-only/);
  assert.equal(await fs.readFile(path.join(toolchain.sdk, "llms.txt"), "utf8"), index);
  await assert.rejects(fs.access(path.join(root, "work", "sdk")));
});

test("a write into the reference fails and the file stays as it is", async () => {
  const toolchain = new Toolchain(plugin, "/unused");
  const index = await fs.realpath(path.join(toolchain.sdk, "llms.txt"));
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

/** The agent's own tools on a fresh work folder, opened as the worker opens them (Folders.open). */
async function agentTools() {
  const root = await dataDirectory();
  await fs.mkdir(path.join(root, "work"));
  const toolchain = new Toolchain(plugin, root);
  const folders = await Folders.open(path.join(root, "work"), toolchain.docsRoots());
  const tools = Object.fromEntries(createTools(folders).map((t) => [t.name, t]));
  const run = async (name, args) => (await tools[name].execute("1", args, undefined, undefined, undefined)).content.map((c) => c.text).join("");
  return { root, run };
}

test("grep names the files it skipped as too large instead of reporting no matches; read refuses a project file over 1 MB", async () => {
  const { root, run } = await agentTools();
  await fs.mkdir(path.join(root, "work", "local.big"));
  await fs.writeFile(path.join(root, "work", "local.big", "huge.txt"), `needle\n${"x".repeat(1024 * 1024)}\n`);
  await fs.writeFile(path.join(root, "work", "local.big", "small.txt"), "hay\n");

  const skipped = "skipped local.big/huge.txt: larger than 1.0 MB";
  assert.equal(await run("grep", { pattern: "needle", path: "local.big" }), `No matches found in the files searched\n${skipped}`);
  assert.equal(await run("grep", { pattern: "needle", path: "local.big/huge.txt" }), `No matches found in the files searched\n${skipped}`);
  // Matches elsewhere come with the note too.
  assert.equal(await run("grep", { pattern: "hay|needle", path: "local.big" }), `local.big/small.txt:1: hay\n${skipped}`);
  assert.equal(await run("grep", { pattern: "nothing", path: "local.big/small.txt" }), "No matches found");
  await assert.rejects(run("read", { path: "local.big/huge.txt", limit: 1 }), /File larger than 1\.0 MB \(1\.0 MB\): local\.big\/huge\.txt/);
});

/** The agent's docs tools, as createTools gives them to the worker. */
async function docsTool() {
  const { run } = await agentTools();
  return run;
}

test("docs: the vendored SDK's tree from the root down to a member", async () => {
  const run = await docsTool();
  const root = await run("docs", {});
  assert.match(root, /^\/ · Vestra Player SDK \d+\.\d+\.\d+ documentation/);
  for (const path of ["/Overview", "/Guides", "/API"]) assert.ok(root.includes(`- ${path}`), path);
  assert.equal(await run("docs", { path: "/" }), root);
  const member = await run("docs", { path: "/API/Media/Snapshot/export" });
  assert.match(member, /^\/API\/Media\/Snapshot\/export · Method/);
  assert.match(member, /export\(options: ExportOptions\): Promise<ExportJob>/);
  // A name alone finds its declaration.
  assert.match(await run("docs", { path: "Snapshot.export" }), /^\/API\/Media\/Snapshot\/export/);
  for (const answer of [root, member]) assert.ok(Buffer.byteLength(answer) <= 8192);
});

test("docs_search: the best match first, and long answers in pages", async () => {
  const run = await docsTool();
  const first = await run("docs_search", { query: "add a menu item" });
  assert.match(first, /^Search: add a menu item · page 1 of (\d+)/);
  const hits = first.split("\n").filter((line) => line.startsWith("- /"));
  assert.match(hits[0], /^- \/API\/Commands\/Commands\/add /);
  const pages = Number(/page 1 of (\d+)/.exec(first)[1]);
  assert.ok(pages > 1);
  const second = await run("docs_search", { query: "add a menu item", page: 2 });
  assert.match(second, /· page 2 of /);
  assert.notEqual(second, first);
  assert.ok(Buffer.byteLength(first) <= 8192 && Buffer.byteLength(second) <= 8192);
  // A bad page number is the first page, not an error.
  assert.equal(await run("docs_search", { query: "add a menu item", page: 0 }), first);
});

test("the docs tools come from the vendored SDK, not from a node_modules folder", async () => {
  const source = await fs.readFile(path.join(plugin, "worker", "docs-tools.ts"), "utf8");
  assert.match(source, /from "\.\.\/vendor\/@vestra-player\/sdk\/dist\/docs\.js"/);
  assert.doesNotMatch(source, /from "@vestra-player\/sdk/);
});
