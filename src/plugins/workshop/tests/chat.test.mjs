// The slice end to end: menu → settings (keychain) → a message → a real agent
// worker → streamed reply → a tool writes a file in the session's work folder.
// The model is a local mock OpenAI-compatible server; no paid service is called.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { KEY, dataDirectory, filesContaining, server, startAi } from "./fixture.mjs";

const writeThenDone = (req, index) =>
  req.body.messages.at(-1).role === "tool"
    ? { text: ["写好了", "：notes.md。"] }
    : { text: ["好的，", "我来写。"], toolCalls: [{ name: "write", args: { path: "notes.md", content: "# 笔记\n" } }], index };

test("one menu entrance; without a key the window says so and links to the settings page", async () => {
  const ai = await startAi();
  assert.deepEqual(ai.host.command("workshop.open").declaration.path, ["@tools", "Vestra Player AI…"]);
  // The player names its own menus (Tools ▸); the test host shows them in Simplified Chinese.
  assert.deepEqual(ai.host.command("workshop.open").display.path, ["工具", "Vestra Player AI…"]);
  await ai.until(() => ai.props.banner.code === "not-configured", "banner");
  assert.equal(ai.banner(), "No provider set up: choose a provider in the settings and save its API key first.");
  assert.equal(ai.host.command("workshop.open").declaration.order, 820);
  assert.equal(ai.window.file, "ai.slint");
  assert.equal(ai.props.configured, false);
  assert.deepEqual(ai.rows(), []);
  await ai.send("open-settings");
  assert.deepEqual(ai.host.settings.shows, ["byok"]);
  const page = ai.host.settings.pages.get("byok");
  assert.equal(page.category, "plugins");
  assert.equal(page.title, "Vestra Player AI");
  await ai.stop();
});

test("Simplified Chinese: the menu, the banner and the settings page read as before", async () => {
  const ai = await startAi({ locale: "zh-Hans" });
  assert.equal(ai.plugin.i18n.locale, "zh-Hans");
  assert.deepEqual(ai.host.command("workshop.open").display.path, ["工具", "Vestra Player AI…"]);
  await ai.until(() => ai.props.banner.code === "not-configured", "banner");
  assert.equal(ai.banner(), "尚未配置服务商：请先在设置中选择服务商并保存 API key。");
  assert.equal(ai.keyState(), "未保存");
  assert.match(ai.ownerHint(), /^没有填写：新插件的 id 用 local\.<名字>/);
  await ai.stop();
});

test("a saved key goes to the keychain only and is shown masked", async () => {
  const ai = await startAi({ locale: "zh-Hans" });
  ai.page.emit("save-key", KEY);
  await ai.until(() => ai.props.configured === true, "configured");
  assert.equal(ai.host.secrets.get("key.anthropic"), KEY);
  assert.equal(ai.keyState(), `已保存 ••••${KEY.slice(-4)}`);
  assert.equal(ai.message(), "Anthropic 的 key 已保存。");
  assert.equal(ai.page.props["key-text"], "");
  assert.equal(ai.modelLine(), "Anthropic · claude-sonnet-4-5");
  assert.deepEqual(await filesContaining(ai.plugin.dataDirectory, KEY), []);
  ai.page.emit("delete-key");
  await ai.until(() => ai.props.configured === false, "key deleted");
  assert.equal(ai.host.secrets.has("key.anthropic"), false);
  await ai.stop();
});

test("a message streams back; the write tool is a step of the activity row and the file lands in work/", async () => {
  const mock = await server(writeThenDone);
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("写一个 notes.md");
  assert.equal(ai.kinds(), "user,activity,assistant");
  const [user, activity, reply] = ai.rows();
  assert.equal(user.text, "写一个 notes.md");
  // 「好的，我来写。」 came before a tool call: a progress note, not the answer, and not shown.
  assert.match(ai.shown(1), /^用时 \d+ 秒 · 1 个步骤$/);
  assert.equal(activity.text, "");
  assert.equal(await ai.calls(1), "✓ 写入 notes.md");
  assert.equal(reply.text, "写好了：notes.md。");

  const [meta] = (await fs.readdir(path.join(ai.plugin.dataDirectory, "sessions"))).map((id) => path.join(ai.plugin.dataDirectory, "sessions", id));
  assert.equal(await fs.readFile(path.join(meta, "work/notes.md"), "utf8"), "# 笔记\n");
  const saved = JSON.parse(await fs.readFile(path.join(meta, "meta.json"), "utf8"));
  assert.equal(saved.title, "写一个 notes.md");
  assert.equal(saved.running, false);
  assert.match(saved.file, /\.jsonl$/);
  assert.equal(ai.props["has-folder"], true);
  assert.equal(ai.props.sessions[1].title, "写一个 notes.md");

  // The key reached the provider, and nothing on disk.
  assert.equal(mock.chats[0].headers.authorization, `Bearer ${KEY}`);
  assert.deepEqual(await filesContaining(ai.plugin.dataDirectory, KEY), []);
  // pi got our system prompt and exactly our tools: the file tools and the plugin tools.
  const body = mock.chats[0].body;
  assert.match(body.messages[0].content, /Vestra Player AI/);
  assert.deepEqual(
    body.tools.map((t) => t.function.name).sort(),
    ["check", "delete", "docs", "docs_search", "edit", "find", "grep", "install_plugin", "ls", "pack", "read", "scaffold_plugin", "test", "typecheck", "write"],
  );

  await ai.send("toggle", activity.id);
  assert.equal(ai.rows()[1].expanded, true);
  await ai.send("copy", reply.id);
  assert.equal(ai.host.clipboard.text, reply.text);
  // The user's own message has a Copy link too.
  await ai.send("copy", user.id);
  assert.equal(ai.host.clipboard.text, "写一个 notes.md");
  await ai.send("open-folder");
  assert.deepEqual(ai.host.system.opened.map((p) => path.basename(p)), ["work"]);
  await ai.stop();
});

test("text appears while it streams", async () => {
  // A reply without tool calls is the answer from its first chunk on.
  const mock = await server(() => ({ text: ["第一段\n\n", "第二段\n\n", "第三段"], pauseMs: 400 }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("讲点什么", { wait: false });
  await ai.until(() => ai.props.busy === true && ai.rows().at(-1)?.text === "第一段\n\n第二段\n\n", "first chunks", 10_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(ai.rows().at(-1).text, "第一段\n\n第二段\n\n第三段");
  await ai.stop();
});

test("Stop ends a run that hangs", async () => {
  const mock = await server(() => ({ hang: true }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("你好", { wait: false });
  await ai.until(() => ai.props.busy === true && mock.chats.length === 1, "request sent");
  await ai.send("stop");
  await ai.until(() => ai.props.busy === false, "stopped");
  assert.equal(ai.rows().at(-1).kind, "notice");
  assert.equal(ai.shown(-1), "已停止");
  await ai.stop();
});

test("an invalid key shows the error row with Open Settings and Retry sends again", async () => {
  let fail = true;
  const mock = await server(() => (fail ? { status: 401, error: "Incorrect API key provided" } : { text: ["好了"] }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("你好");
  const error = ai.rows().at(-1);
  assert.equal(error.kind, "error");
  assert.equal(ai.shown(-1), "API key 无效或没有权限，请在设置中检查。");
  assert.equal(error.settings, true);
  assert.equal(ai.props["can-retry"], true);
  fail = false;
  await ai.send("retry");
  await ai.until(() => ai.props.busy === false && ai.rows().at(-1).kind === "assistant", "retried");
  assert.equal(ai.rows().at(-1).text, "好了");
  await ai.stop();
});

test("an unreachable endpoint is a network error after pi's retries", async () => {
  const ai = await startAi({ locale: "zh-Hans" });
  // Nothing listens there: the model list fails too, so the id is typed.
  await ai.configure("http://127.0.0.1:9/v1", { model: "mock-1" });
  await ai.ask("你好");
  await ai.until(() => ai.rows().at(-1)?.kind === "error", "error", 30_000);
  assert.equal(ai.shown(-1), "网络错误：无法连接服务商。");
  assert.equal(ai.rows().at(-1).settings, false);
  await ai.stop();
});

test("new chat, switch back, and the conversation survives a restart", async () => {
  const mock = await server((req) => ({ text: [`回答 ${req.body.messages.filter((m) => m.role === "user").length}`] }));
  const data = await dataDirectory();
  let ai = await startAi({ data, compat: mock.url });
  await ai.ask("第一个会话");
  await ai.send("new-chat");
  assert.deepEqual(ai.rows(), []);
  assert.equal(ai.props["session-title"], "", "a new conversation (the window shows New chat)");
  await ai.ask("第二个会话");
  assert.deepEqual(
    ai.props.sessions.filter((s) => !s.header).map((s) => [s.title, s.active]),
    [
      ["第二个会话", true],
      ["第一个会话", false],
    ],
  );
  const first = ai.props.sessions.find((s) => s.title === "第一个会话");
  await ai.send("select-session", first.id);
  await ai.until(() => ai.rows()[0]?.text === "第一个会话", "switched");
  assert.equal(ai.rows()[1].text, "回答 1");
  const secrets = Object.fromEntries(ai.host.secrets);
  await ai.stop();

  // Restart: the last shown session comes back from pi's JSONL and continues.
  ai = await startAi({ data, secrets });
  await ai.until(() => ai.rows().length === 2, "restored");
  assert.equal(ai.props["session-title"], "第一个会话");
  assert.deepEqual(ai.rows().map((r) => r.text), ["第一个会话", "回答 1"]);
  await ai.ask("接着聊");
  assert.equal(ai.rows().at(-1).text, "回答 2");
  const text = (content) => (typeof content === "string" ? content : content.map((part) => part.text).join(""));
  const history = mock.chats.at(-1).body.messages.filter((m) => m.role !== "system").map((m) => text(m.content));
  assert.deepEqual(history.slice(0, 2), ["第一个会话", "回答 1"]);
  await ai.stop();
});
