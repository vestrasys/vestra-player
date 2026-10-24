// Conversation rows: the rolling tool line, errors, and rebuilding from pi's JSONL.
import assert from "node:assert/strict";
import test from "node:test";
import { Chat, classify } from "../chat.ts";

test("tool calls share one line: the latest replaces the previous, the list expands", () => {
  const chat = new Chat();
  chat.user("做个插件");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "tool_start", id: "a", name: "write", target: "main.ts" });
  assert.equal(chat.rows[1].text, "写入 main.ts…");
  chat.apply({ kind: "tool_end", id: "a", name: "write", ok: true, summary: "" });
  chat.apply({ kind: "tool_start", id: "b", name: "read", target: "manifest.json" });
  assert.equal(chat.rows.length, 2, "still one tool line");
  assert.equal(chat.rows[1].text, "读取 manifest.json…");
  assert.equal(chat.rows[1].state, 0);
  chat.apply({ kind: "tool_end", id: "b", name: "read", ok: false, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "完成" });
  chat.apply({ kind: "assistant_end", text: "完成", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, r.text, r.state]),
    [
      ["user", "做个插件", 1],
      ["tools", "2 个工具调用，1 个失败", 2],
      ["assistant", "完成", 1],
    ],
  );
  assert.equal(chat.rows[1].detail, "✓ 写入 main.ts\n✗ 读取 manifest.json");
  chat.toggle(chat.rows[1].id);
  assert.equal(chat.rows[1].expanded, true);
});

test("the tool line sits before text that came first in the turn", () => {
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "text", delta: "先说一句" });
  chat.apply({ kind: "tool_start", id: "a", name: "ls", target: "" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "tools", "assistant"]);
});

test("errors: key, quota, network; an automatic retry replaces the error row", () => {
  assert.equal(classify("401 Incorrect API key provided").settings, true);
  assert.match(classify("429 You exceeded your current quota").text, /额度/);
  assert.match(classify("fetch failed: ECONNREFUSED").text, /网络/);
  const chat = new Chat();
  chat.user("hi");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "Connection error." });
  assert.equal(chat.rows.at(-1).kind, "error");
  chat.apply({ kind: "retry", attempt: 1, max: 2, error: "Connection error." });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "notice"]);
});

test("rows from pi's JSONL", () => {
  const lines = [
    { type: "session", version: 3, id: "x", cwd: "/w" },
    { type: "message", message: { role: "user", content: [{ type: "text", text: "写文件" }] } },
    { type: "message", message: { role: "assistant", content: [{ type: "text", text: "好" }, { type: "toolCall", id: "c1", name: "write", arguments: { path: "a.md", content: "x" } }], stopReason: "toolUse" } },
    { type: "message", message: { role: "toolResult", toolCallId: "c1", toolName: "write", content: [], isError: false } },
    { type: "message", message: { role: "assistant", content: [{ type: "text", text: "写好了" }], stopReason: "stop" } },
    { type: "message", message: { role: "user", content: "再来" } },
    { type: "message", message: { role: "assistant", content: [], stopReason: "error", errorMessage: "401 invalid x-api-key" } },
  ];
  const chat = Chat.fromJsonl(`${lines.map((l) => JSON.stringify(l)).join("\n")}\n{"type":"mess`);
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, r.text]),
    [
      ["user", "写文件"],
      ["tools", "1 个工具调用"],
      ["assistant", "好\n\n写好了"],
      ["user", "再来"],
      ["error", "API key 无效或没有权限，请在设置中检查。"],
    ],
  );
  assert.equal(chat.lastPrompt, "再来");
});
