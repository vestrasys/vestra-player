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
  assert.deepEqual(ai.host.command("workshop.open").declaration.path, ["Vestra Player AI…"]);
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

test("a saved key goes to the keychain only and is shown masked", async () => {
  const ai = await startAi();
  ai.page.emit("save-key", KEY);
  await ai.until(() => ai.props.configured === true, "configured");
  assert.equal(ai.host.secrets.get("key.anthropic"), KEY);
  assert.equal(ai.page.props["key-state"], `已保存 ••••${KEY.slice(-4)}`);
  assert.equal(ai.page.props["key-text"], "");
  assert.equal(ai.props["model-text"], "Anthropic · claude-sonnet-4-5");
  assert.deepEqual(await filesContaining(ai.plugin.dataDirectory, KEY), []);
  ai.page.emit("delete-key");
  await ai.until(() => ai.props.configured === false, "key deleted");
  assert.equal(ai.host.secrets.has("key.anthropic"), false);
  await ai.stop();
});

test("a message streams back; the write tool shows one tool line and the file lands in work/", async () => {
  const mock = await server(writeThenDone);
  const ai = await startAi({ compat: mock.url });
  await ai.ask("写一个 notes.md");
  assert.equal(ai.kinds(), "user,tools,assistant");
  const [user, tools, reply] = ai.rows();
  assert.equal(user.text, "写一个 notes.md");
  assert.equal(tools.text, "1 个工具调用");
  assert.equal(tools.detail, "✓ 写入 notes.md");
  assert.equal(reply.text, "好的，我来写。\n\n写好了：notes.md。");

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
    ["check", "delete", "edit", "find", "grep", "install_plugin", "ls", "pack", "read", "scaffold_plugin", "test", "typecheck", "write"],
  );

  await ai.send("toggle", tools.id);
  assert.equal(ai.rows()[1].expanded, true);
  await ai.send("copy", reply.id);
  assert.equal(ai.host.clipboard.text, reply.text);
  await ai.send("open-folder");
  assert.deepEqual(ai.host.system.opened.map((p) => path.basename(p)), ["work"]);
  await ai.stop();
});

test("text appears while it streams", async () => {
  const mock = await server(() => ({ text: ["第一段", "第二段", "第三段"], pauseMs: 400 }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("讲点什么", { wait: false });
  await ai.until(() => ai.props.busy === true && ai.rows().at(-1)?.text === "第一段", "first chunk", 10_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(ai.rows().at(-1).text, "第一段第二段第三段");
  await ai.stop();
});

test("「停止」 ends a run that hangs", async () => {
  const mock = await server(() => ({ hang: true }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("你好", { wait: false });
  await ai.until(() => ai.props.busy === true && mock.chats.length === 1, "request sent");
  await ai.send("stop");
  await ai.until(() => ai.props.busy === false, "stopped");
  assert.equal(ai.rows().at(-1).kind, "notice");
  assert.equal(ai.rows().at(-1).text, "已停止");
  await ai.stop();
});

test("an invalid key shows the error row with 「打开设置」 and 「重试」 sends again", async () => {
  let fail = true;
  const mock = await server(() => (fail ? { status: 401, error: "Incorrect API key provided" } : { text: ["好了"] }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("你好");
  const error = ai.rows().at(-1);
  assert.equal(error.kind, "error");
  assert.match(error.text, /API key 无效/);
  assert.equal(error.settings, true);
  assert.equal(ai.props["can-retry"], true);
  fail = false;
  await ai.send("retry");
  await ai.until(() => ai.props.busy === false && ai.rows().at(-1).kind === "assistant", "retried");
  assert.equal(ai.rows().at(-1).text, "好了");
  await ai.stop();
});

test("an unreachable endpoint is a network error after pi's retries", async () => {
  const ai = await startAi();
  // Nothing listens there: the model list fails too, so the id is typed.
  await ai.configure("http://127.0.0.1:9/v1", { model: "mock-1" });
  await ai.ask("你好");
  await ai.until(() => ai.rows().at(-1)?.kind === "error", "error", 30_000);
  assert.match(ai.rows().at(-1).text, /网络错误/);
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
  assert.equal(ai.props["session-title"], "新对话");
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
