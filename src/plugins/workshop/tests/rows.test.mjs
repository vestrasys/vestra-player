// Conversation rows: the rolling tool line, errors, and rebuilding from pi's JSONL.
import assert from "node:assert/strict";
import test from "node:test";
import { Chat, classify } from "../chat.ts";
import { error, notice, toolLine, toolList, translator } from "./texts.mjs";

const zh = translator("zh-Hans");
const en = translator("en");
/** A row as the window shows it in Simplified Chinese: the message, or the line its code makes. */
const shown = (row) => (row.kind === "tools" ? toolLine(zh, row) : row.kind === "error" ? error(zh, row) : row.text);

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

test("the tool line sits before text that came first in the turn", () => {
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "text", delta: "先说一句。\n" });
  chat.apply({ kind: "text", delta: "再说一句。" });
  chat.apply({ kind: "tool_start", id: "a", name: "ls", target: "" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "tools", "assistant"]);
});

test("an opening line before the turn's first tool call is not shown; later text and plain replies are", () => {
  const chat = new Chat();
  chat.user("写个插件");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "I'll start by reading " });
  chat.apply({ kind: "text", delta: "the SDK documentation.\n\n" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user"], "held back while it is one line");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "a", name: "read", target: "sdk/llms.txt" });
  chat.apply({ kind: "tool_end", id: "a", name: "read", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "测试失败是因为绑定写法不对。" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "b", name: "edit", target: "main.ts" });
  chat.apply({ kind: "tool_end", id: "b", name: "edit", ok: true, summary: "" });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => [r.kind, shown(r)]), [
    ["user", "写个插件"],
    ["tools", "2 个工具调用"],
    ["assistant", "测试失败是因为绑定写法不对。"],
  ]);
  // A reply without tool calls is shown, short or long; a long first line streams at once.
  chat.user("你好");
  chat.apply({ kind: "text", delta: "你好！" });
  chat.apply({ kind: "assistant_end", text: "你好！", stopReason: "stop", error: null });
  assert.equal(chat.rows.at(-1).text, "你好！");
  chat.user("长一点");
  chat.apply({ kind: "text", delta: "很长".repeat(120) });
  assert.equal(chat.rows.at(-1).kind, "assistant");
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
    { type: "message", message: { role: "assistant", content: [{ type: "text", text: "I'll write the file." }, { type: "toolCall", id: "c1", name: "write", arguments: { path: "a.md", content: "x" } }], stopReason: "toolUse" } },
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
      ["tools", "1 个工具调用"],
      ["assistant", "写好了"],
      ["user", "再来"],
      ["error", "API key 无效或没有权限，请在设置中检查。"],
    ],
  );
  assert.equal(chat.lastPrompt, "再来");
});
