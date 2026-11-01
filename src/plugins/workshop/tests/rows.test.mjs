// Conversation rows: the rolling tool line, errors, and rebuilding from pi's JSONL.
import assert from "node:assert/strict";
import test from "node:test";
import { Chat, classify } from "../chat.ts";
import { MessageDocs } from "../messages.ts";
import { error, notice, thoughtLine, toolLine, toolList, translator } from "./texts.mjs";

const zh = translator("zh-Hans");
const en = translator("en");
/** A row as the window shows it in Simplified Chinese: the message, or the line its code makes. */
const shown = (row) => (row.kind === "tools" ? toolLine(zh, row) : row.kind === "thought" ? thoughtLine(zh, row) : row.kind === "error" ? error(zh, row) : row.text);

test("tool calls share one line: the latest replaces the previous, the list expands", () => {
  const chat = new Chat();
  chat.user("做个插件");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "tool_start", id: "a", name: "write", target: "main.ts" });
  assert.equal(toolLine(zh, chat.rows[1]), "写入 main.ts…");
  assert.equal(toolLine(en, chat.rows[1]), "Write main.ts…");
  chat.apply({ kind: "tool_end", id: "a", name: "write", ok: true, summary: "" });
  chat.apply({ kind: "tool_start", id: "b", name: "read", target: "manifest.json" });
  assert.equal(chat.rows.length, 2, "still one tool line");
  assert.equal(toolLine(zh, chat.rows[1]), "读取 manifest.json…");
  assert.equal(chat.rows[1].state, 0);
  chat.apply({ kind: "tool_end", id: "b", name: "read", ok: false, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "完成" });
  chat.apply({ kind: "assistant_end", text: "完成", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, shown(r), r.state]),
    [
      ["user", "做个插件", 1],
      ["tools", "2 个工具调用，1 个失败", 2],
      ["assistant", "完成", 1],
    ],
  );
  assert.equal(toolLine(en, chat.rows[1]), "2 tool calls, 1 failed");
  assert.equal(toolList(zh, chat.rows[1]), "✓ 写入 main.ts\n✗ 读取 manifest.json");
  chat.toggle(chat.rows[1].id);
  assert.equal(chat.rows[1].expanded, true);
});

test("text before a tool call is a progress note: it leaves the answer for the collapsed thought row, before the tool line", () => {
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "text", delta: "先说一句。\n" });
  chat.apply({ kind: "text", delta: "再说一句。" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "assistant"], "shown while it streams");
  chat.apply({ kind: "tool_start", id: "a", name: "ls", target: "" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "thought", "tools"]);
  assert.equal(chat.rows[1].text, "先说一句。\n再说一句。");
  assert.equal(chat.rows[1].expanded, false);
  assert.equal(thoughtLine(zh, chat.rows[1]), "1 条进度说明");
});

test("notes between tool calls go to the thought row; the answer is what the model wrote without a tool call", () => {
  const chat = new Chat();
  chat.user("写个插件");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "I'll start by reading " });
  chat.apply({ kind: "text", delta: "the SDK documentation.\n\n" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "a", name: "docs", target: "/" });
  chat.apply({ kind: "tool_end", id: "a", name: "read", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "测试失败是因为绑定写法不对。" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "b", name: "edit", target: "main.ts" });
  chat.apply({ kind: "tool_end", id: "b", name: "edit", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "插件做好了。" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, r.kind === "thought" ? thoughtLine(zh, r) : shown(r)]),
    [
      ["user", "写个插件"],
      ["thought", "2 条进度说明"],
      ["tools", "2 个工具调用"],
      ["assistant", "插件做好了。"],
    ],
  );
  assert.equal(chat.rows[1].text, "I'll start by reading the SDK documentation.\n\n测试失败是因为绑定写法不对。");
  assert.equal(thoughtLine(en, chat.rows[1]), "2 progress notes");
  // A reply without tool calls is the answer, short or long.
  chat.user("你好");
  chat.apply({ kind: "text", delta: "你好！" });
  chat.apply({ kind: "assistant_end", text: "你好！", stopReason: "stop", error: null });
  assert.deepEqual(chat.rows.slice(-2).map((r) => [r.kind, r.text]), [["user", "你好"], ["assistant", "你好！"]]);
});

test("thinking shows as one collapsed row, timed while live: Thinking…, then Thought for n seconds", () => {
  let clock = 1000;
  const chat = new Chat(() => clock);
  chat.user("做个插件");
  chat.begin();
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "The user wants " });
  chat.apply({ kind: "thinking", delta: "a plugin." });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "thought", "tools"]);
  assert.equal(thoughtLine(zh, chat.rows[1]), "思考中…");
  assert.equal(chat.rows[1].state, 0);
  clock += 4000;
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "a", name: "docs", target: "/" });
  chat.apply({ kind: "tool_end", id: "a", name: "read", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "Now the code." });
  clock += 8000;
  chat.apply({ kind: "text", delta: "好了。" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "thought", "tools", "assistant"]);
  assert.equal(chat.rows[1].state, 1);
  assert.equal(thoughtLine(en, chat.rows[1]), "Thought for 12 seconds");
  assert.equal(chat.rows[1].text, "The user wants a plugin.\n\nNow the code.");
  assert.equal(chat.rows[3].text, "好了。", "no thinking in the answer");
  chat.toggle(chat.rows[1].id);
  assert.equal(chat.rows[1].expanded, true);
});

test("a collapsed thought row sends no text to the window; expanded, at most 16 KB of it", async () => {
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "想".repeat(100_000) });
  chat.apply({ kind: "text", delta: "好" });
  chat.apply({ kind: "settled", aborted: false });
  const docs = new MessageDocs(async (text) => [text]);
  await docs.sync(chat.rows);
  const row = () => docs.props(chat.rows).find((r) => r.kind === "thought");
  assert.equal(row().text, "");
  assert.equal(row().truncated, false);
  chat.toggle(chat.rows[1].id);
  assert.ok(Buffer.byteLength(row().text) <= 16 * 1024);
  assert.equal(row().truncated, true);
});

test("errors: key, quota, network; an automatic retry replaces the error row", () => {
  assert.equal(classify("401 Incorrect API key provided").settings, true);
  assert.equal(classify("429 You exceeded your current quota").code, "quota");
  assert.equal(classify("fetch failed: ECONNREFUSED").code, "network");
  assert.equal(error(zh, classify("429 You exceeded your current quota")), "额度不足或请求过于频繁，请稍后重试。");
  assert.equal(error(zh, classify("fetch failed: ECONNREFUSED")), "网络错误：无法连接服务商。");
  assert.equal(error(en, classify("fetch failed: ECONNREFUSED")), "Network error: cannot reach the provider.");
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "Connection error." });
  assert.equal(chat.rows.at(-1).kind, "error");
  chat.apply({ kind: "retry", attempt: 1, max: 2, error: "Connection error." });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "notice"]);
  assert.equal(notice(zh, chat.rows[1]), "连接失败，正在重试（1/2）…");
});

test("rows from pi's JSONL", () => {
  const lines = [
    { type: "session", version: 3, id: "x", cwd: "/w" },
    { type: "message", message: { role: "user", content: [{ type: "text", text: "写文件" }] } },
    { type: "message", message: { role: "assistant", content: [{ type: "thinking", thinking: "Write a.md." }, { type: "text", text: "I'll write the file." }, { type: "toolCall", id: "c1", name: "write", arguments: { path: "a.md", content: "x" } }], stopReason: "toolUse" } },
    { type: "message", message: { role: "toolResult", toolCallId: "c1", toolName: "write", content: [], isError: false } },
    { type: "message", message: { role: "assistant", content: [{ type: "text", text: "写好了" }], stopReason: "stop" } },
    { type: "message", message: { role: "user", content: "再来" } },
    { type: "message", message: { role: "assistant", content: [], stopReason: "error", errorMessage: "401 invalid x-api-key" } },
  ];
  const chat = Chat.fromJsonl(`${lines.map((l) => JSON.stringify(l)).join("\n")}\n{"type":"mess`);
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, shown(r)]),
    [
      ["user", "写文件"],
      ["thought", "思考过程"],
      ["tools", "1 个工具调用"],
      ["assistant", "写好了"],
      ["user", "再来"],
      ["error", "API key 无效或没有权限，请在设置中检查。"],
    ],
  );
  assert.equal(chat.rows[1].text, "Write a.md.\n\nI'll write the file.");
  assert.equal(chat.rows[1].expanded, false, "older sessions' thinking is collapsed too");
  assert.equal(chat.lastPrompt, "再来");
});

test("a turn that ends without an answer shows its last note as the answer: summary with the last tool call, Stop, an error", () => {
  const note = (chat, text, id) => {
    chat.apply({ kind: "assistant_start" });
    chat.apply({ kind: "text", delta: text });
    chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
    chat.apply({ kind: "tool_start", id, name: "check", target: "local.demo" });
    chat.apply({ kind: "tool_end", id, name: "check", ok: true, summary: "" });
  };
  const chat = new Chat();
  chat.user("做个插件");
  note(chat, "先检查。", "a");
  note(chat, "插件做好了，可以安装。", "b");
  // The model's last message is empty.
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => [r.kind, r.kind === "thought" ? thoughtLine(zh, r) : shown(r)]), [
    ["user", "做个插件"],
    ["thought", "1 条进度说明"],
    ["tools", "2 个工具调用"],
    ["assistant", "插件做好了，可以安装。"],
  ]);
  assert.equal(chat.rows[1].text, "先检查。");

  // Stop after one note: the note is the answer, and no thought row stays empty.
  chat.user("再改改");
  note(chat, "改好了一半。", "c");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "aborted", error: null });
  assert.deepEqual(chat.rows.slice(4).map((r) => [r.kind, r.kind === "notice" ? notice(zh, r) : shown(r)]), [
    ["user", "再改改"],
    ["tools", "1 个工具调用"],
    ["assistant", "改好了一半。"],
    ["notice", "已停止"],
  ]);

  // An error after a note: the note, then the error.
  chat.user("继续");
  note(chat, "正在测试。", "d");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "Connection error." });
  assert.deepEqual(chat.rows.slice(-3).map((r) => r.kind), ["tools", "assistant", "error"]);
  assert.equal(chat.rows.at(-2).text, "正在测试。");
});

test("an endpoint that refuses the reasoning parameters: an error that points to Settings ▸ Thinking", () => {
  for (const message of [
    "400 Unrecognized request argument supplied: reasoning_effort",
    "400 thinking: Extra inputs are not permitted",
    "400 invalid value for budget_tokens",
    "400 This model does not support thinking",
  ]) {
    const found = classify(message, "m");
    assert.equal(found.code, "thinking", message);
    assert.equal(found.settings, true);
  }
  assert.equal(classify("401 invalid x-api-key").code, "key");
  assert.equal(
    error(zh, classify("400 This model does not support thinking")),
    "模型不接受思考设置：请在设置中把「思考」设为「关闭」，或取消勾选「这个模型会先思考再回答」。",
  );
});

test("the last note becomes the answer also when thinking came after it: it leaves the thought row, the thinking stays", () => {
  const chat = new Chat();
  chat.user("做个插件");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "Plan it." });
  chat.apply({ kind: "text", delta: "插件做好了。" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "a", name: "check", target: "local.demo" });
  chat.apply({ kind: "tool_end", id: "a", name: "check", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "Done, nothing to add." });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "thought", "tools", "assistant"]);
  assert.equal(chat.rows[1].text, "Plan it.\n\nDone, nothing to add.");
  assert.equal(chat.rows[3].text, "插件做好了。");
});
