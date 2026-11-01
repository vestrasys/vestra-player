// Deleting a conversation, as chat apps do: a button on the row (and Delete
// Chat for the open one), a confirmation, then its folder goes (record, plugin
// projects, packed files); a run in it stops first; what it installed stays.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { server, startAi } from "./fixture.mjs";

const titles = (ai) => ai.props.sessions.filter((s) => !s.header).map((s) => s.title);
const folders = async (ai) => (await fs.readdir(path.join(ai.plugin.dataDirectory, "sessions"))).sort();
const exists = (file) => fs.access(file).then(() => true, () => false);

test("a conversation is deleted after the confirmation; Cancel keeps it, the open one stays open", async () => {
  const mock = await server(() => ({ text: ["好"] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("第一个");
  await ai.send("new-chat");
  await ai.ask("第二个");
  const first = ai.props.sessions.find((s) => s.title === "第一个");
  const directory = path.join(ai.plugin.dataDirectory, "sessions", first.id);
  await fs.mkdir(path.join(directory, "dist"));
  await fs.writeFile(path.join(directory, "dist", "local.demo-0.1.0.vxp"), "x");

  ai.host.dialogAnswers.push({ confirmed: false, checked: false });
  await ai.send("delete-session", first.id);
  assert.deepEqual(ai.host.confirmTexts.at(-1), {
    message: "删除对话「第一个」？",
    detail: "对话记录和其中做的插件项目（源码文件夹和打包好的 .vxp 文件）都会删除；你导出的文件不受影响。",
    confirm: "删除",
    cancel: ai.host.confirmTexts.at(-1).cancel,
  });
  assert.equal(ai.host.confirms.at(-1).destructive, true);
  assert.deepEqual(titles(ai), ["第二个", "第一个"]);
  assert.ok(await exists(directory));

  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-session", first.id);
  await ai.until(() => titles(ai).length === 1, "deleted");
  assert.deepEqual(titles(ai), ["第二个"]);
  assert.equal(ai.props["session-title"], "第二个", "the open conversation stays open");
  assert.equal(await exists(directory), false, "record, work/ and dist/ are gone");
  assert.equal((await folders(ai)).length, 1);
  await ai.stop();
});

test("deleting the open conversation while the agent runs in it: the run stops, a new empty chat shows, nothing comes back", async () => {
  const mock = await server(() => ({ hang: true }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("一直等", { wait: false });
  await ai.until(() => ai.props.busy === true && mock.chats.length === 1, "running");
  const [open] = ai.props.sessions.filter((s) => !s.header);
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-chat");
  await ai.until(() => titles(ai).length === 0 && ai.props.busy === false, "deleted");
  assert.deepEqual(ai.rows(), []);
  assert.equal(ai.props["session-title"], "");
  assert.equal(ai.props["has-folder"], false);
  // The worker is gone and saved nothing afterwards (meta.json would bring the session back).
  await new Promise((resolve) => setTimeout(resolve, 500));
  assert.equal(await exists(path.join(ai.plugin.dataDirectory, "sessions", open.id)), false);
  await ai.stop();
});

test("a plugin installed from the conversation stays installed, and the confirmation says so", async () => {
  const mock = await server(() => ({ text: ["好"] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  const [open] = ai.props.sessions.filter((s) => !s.header);
  const project = path.join(ai.plugin.dataDirectory, "sessions", open.id, "work", "local.demo");
  await fs.mkdir(project, { recursive: true });
  await fs.writeFile(path.join(project, "manifest.json"), JSON.stringify({ id: "local.demo", name: "演示", version: "0.1.0" }));
  ai.host.packages.push({
    id: "local.demo",
    version: "0.1.0",
    origin: "installed",
    state: "running",
    error: null,
    capabilities: [],
    directory: "/packages/x",
    source: { kind: "workshop" },
  });
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-session", open.id);
  await ai.until(() => titles(ai).length === 0, "deleted");
  assert.equal(
    ai.host.confirmTexts.at(-1).detail,
    "对话记录和其中做的插件项目（源码文件夹和打包好的 .vxp 文件）都会删除；你导出的文件不受影响。从这个对话安装的插件仍留在播放器中（local.demo）；不再需要的话，请在插件管理中移除。",
  );
  assert.deepEqual(ai.host.packageCalls, [], "nothing uninstalled");
  assert.ok(ai.host.packages.some((p) => p.id === "local.demo"));
  await ai.stop();
});
