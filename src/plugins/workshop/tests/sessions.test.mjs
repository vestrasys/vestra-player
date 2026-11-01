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

/** The player's validation of a packed project (as in delivery.test.mjs): accepts it with its manifest. */
function player(ai) {
  ai.host.handle("packages.validateArchive", async ({ file }) => {
    const id = path.basename(String(file)).replace(/-\d+\.\d+\.\d+\.vxp$/, "");
    const manifest = JSON.parse(await fs.readFile(path.join(path.dirname(path.dirname(String(file))), "work", id, "manifest.json"), "utf8"));
    ai.host.candidates.set(String(file), { ...manifest, entry: "main.js" });
    return { token: String(file), digest: "0".repeat(64), apiVersion: "vestra/1", manifest: { ...manifest, entry: "main.js" }, checks: ["manifest"], trustedExecution: true };
  });
}

const PLAIN = "对话记录和其中做的插件项目（源码文件夹和打包好的 .vxp 文件）都会删除；你导出的文件不受影响。";

test("a plugin this conversation installed stays installed and the confirmation names it; one with the same id from elsewhere does not count", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).role === "tool" ? { text: ["好了"] } : { toolCalls: [{ name: "scaffold_plugin", args: { name: "demo", title: "演示" } }] },
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("做个插件");
  await ai.until(() => ai.props["has-project"] === true, "card");
  const [made] = ai.props.sessions.filter((s) => !s.header);

  // Another conversation with a project of the same id, never installed from there.
  await ai.send("new-chat");
  await ai.ask("再做一个");
  const [other] = ai.props.sessions.filter((s) => !s.header);

  // Installed from the first one (the card's Install).
  await ai.send("select-session", made.id);
  await ai.until(() => ai.props["has-project"] === true, "card of the first");
  player(ai);
  await ai.send("install");
  await ai.until(() => ai.props["card-status"].code === "installed", "installed");

  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-session", other.id);
  await ai.until(() => titles(ai).length === 1, "other deleted");
  assert.equal(ai.host.confirmTexts.at(-1).detail, PLAIN, "the same id, but not installed from this conversation");

  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-session", made.id);
  await ai.until(() => titles(ai).length === 0, "deleted");
  assert.equal(ai.host.confirmTexts.at(-1).detail, `${PLAIN}从这个对话安装的插件仍留在播放器中（local.demo）；不再需要的话，请在插件管理中移除。`);
  assert.deepEqual(ai.host.packageCalls.map((c) => c.method), ["install"], "nothing uninstalled");
  assert.ok(ai.host.packages.some((p) => p.id === "local.demo"));
  await ai.stop();
});

test("a delete that fails says so in the conversation, and the conversation stays listed", async (t) => {
  const mock = await server(() => ({ text: ["好"] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("第一个");
  await ai.send("new-chat");
  await ai.ask("第二个");
  const first = ai.props.sessions.find((s) => s.title === "第一个");
  t.mock.method(fs, "rm", async () => {
    throw Object.assign(new Error("EBUSY: resource busy or locked, rmdir"), { code: "EBUSY" });
  });
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-session", first.id);
  await ai.until(() => ai.rows().at(-1)?.kind === "notice", "notice");
  assert.equal(ai.shown(-1), "无法删除「第一个」：EBUSY: resource busy or locked, rmdir");
  assert.deepEqual(titles(ai), ["第二个", "第一个"]);
  t.mock.restoreAll();
  // It can be opened and deleted again.
  await ai.send("select-session", first.id);
  await ai.until(() => ai.rows()[0]?.text === "第一个", "opened");
  ai.host.dialogAnswers.push({ confirmed: true, checked: false });
  await ai.send("delete-chat");
  await ai.until(() => titles(ai).length === 1, "deleted");
  await ai.stop();
});
