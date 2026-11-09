// Conversation rows: the turn's activity row, errors, and rebuilding from pi's JSONL.
import assert from "node:assert/strict";
import test from "node:test";
import { Chat, classify } from "../chat.ts";
import { MessageDocs } from "../messages.ts";
import { activityLine, error, notice, stepList, translator } from "./texts.mjs";

const zh = translator("zh-Hans");
const en = translator("en");
/** A row as the window shows it in Simplified Chinese: the message, or the line its code makes. */
const shown = (row) => (row.kind === "activity" ? activityLine(zh, row) : row.kind === "error" ? error(zh, row) : row.kind === "notice" ? notice(zh, row) : row.text);

test("one activity row per turn: Working… at once, the latest step while it runs, a summary after; the steps expand", () => {
  let clock = 1000;
  const chat = new Chat(() => clock);
  chat.user("做个插件");
  chat.begin();
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity"], "before the model's first event");
  assert.equal(activityLine(zh, chat.rows[1]), "处理中…");
  assert.equal(activityLine(en, chat.rows[1]), "Working…");
  assert.equal(chat.rows[1].state, 0);
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "The user wants a plugin." });
  assert.equal(activityLine(en, chat.rows[1]), "Working…", "thinking is not shown");
  chat.apply({ kind: "tool_start", id: "a", name: "write", target: "main.ts" });
  assert.equal(activityLine(zh, chat.rows[1]), "处理中… · 写入 main.ts…");
  assert.equal(activityLine(en, chat.rows[1]), "Working… · Write main.ts…");
  chat.apply({ kind: "tool_end", id: "a", name: "write", ok: true, summary: "" });
  assert.equal(activityLine(en, chat.rows[1]), "Working… · Write main.ts ✓", "no model phase after the step");
  chat.apply({ kind: "tool_start", id: "b", name: "read", target: "manifest.json" });
  assert.equal(chat.rows.length, 2, "still one activity row");
  assert.equal(activityLine(zh, chat.rows[1]), "处理中… · 读取 manifest.json…");
  chat.apply({ kind: "tool_end", id: "b", name: "read", ok: false, summary: "" });
  assert.equal(activityLine(en, chat.rows[1]), "Working… · Read manifest.json ✗");
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "完成" });
  chat.apply({ kind: "assistant_end", text: "完成", stopReason: "stop", error: null });
  clock += 12_000;
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, shown(r), r.state]),
    [
      ["user", "做个插件", 1],
      ["activity", "用时 12 秒 · 2 个步骤，1 个失败", 2],
      ["assistant", "完成", 1],
    ],
  );
  assert.equal(activityLine(en, chat.rows[1]), "Worked for 12 seconds · 2 steps, 1 failed");
  assert.equal(stepList(zh, chat.rows[1]), "✓ 写入 main.ts\n✗ 读取 manifest.json");
  chat.toggle(chat.rows[1].id);
  assert.equal(chat.rows[1].expanded, true);
});

test("a reply without steps or thinking keeps no activity row; one that thought keeps how long it worked", () => {
  let clock = 0;
  const chat = new Chat(() => clock);
  chat.user("你好");
  chat.begin();
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "你好！" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity", "assistant"], "Working… while the answer streams");
  chat.apply({ kind: "assistant_end", text: "你好！", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => [r.kind, r.text]), [["user", "你好"], ["assistant", "你好！"]]);

  chat.user("想一想");
  chat.begin();
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "thinking", delta: "Hmm." });
  clock += 3400;
  chat.apply({ kind: "text", delta: "好。" });
  chat.apply({ kind: "assistant_end", text: "好。", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.slice(2).map((r) => [r.kind, shown(r)]), [["user", "想一想"], ["activity", "用时 3 秒"], ["assistant", "好。"]]);
  assert.equal(activityLine(en, chat.rows[3]), "Worked for 3 seconds");
  assert.equal(chat.rows[3].n, 0, "nothing to expand");

  // Stopped before the model said anything: no row is left behind.
  chat.user("算了");
  chat.begin();
  chat.apply({ kind: "assistant_end", text: "", stopReason: "aborted", error: null });
  assert.deepEqual(chat.rows.slice(5).map((r) => r.kind), ["user", "notice"]);
});

test("text before a tool call is a progress note: shown while it streams, then not shown; the answer is what the model wrote without a tool call", () => {
  const chat = new Chat();
  chat.user("写个插件");
  chat.begin();
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "I'll start by reading " });
  chat.apply({ kind: "text", delta: "the SDK documentation.\n\n" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity", "assistant"], "shown while it streams");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "toolUse", error: null });
  chat.apply({ kind: "tool_start", id: "a", name: "docs", target: "/" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity"]);
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
    chat.rows.map((r) => [r.kind, r.kind === "activity" ? r.n : r.text]),
    [
      ["user", "写个插件"],
      ["activity", 2],
      ["assistant", "插件做好了。"],
    ],
  );
  assert.equal(chat.rows[1].text, "", "no notes in the activity row");
});

test("an expanded activity row sends its steps to the window, a collapsed one none", async () => {
  const chat = new Chat();
  chat.user("hi");
  chat.begin();
  chat.apply({ kind: "thinking", delta: "想".repeat(100_000) });
  chat.apply({ kind: "tool_start", id: "a", name: "ls", target: "" });
  chat.apply({ kind: "tool_end", id: "a", name: "ls", ok: true, summary: "" });
  chat.apply({ kind: "text", delta: "好" });
  chat.apply({ kind: "settled", aborted: false });
  const docs = new MessageDocs(async (text) => [text]);
  await docs.sync(chat.rows);
  const row = () => docs.props(chat.rows).find((r) => r.kind === "activity");
  assert.equal(row().text, "", "thinking is not sent");
  assert.deepEqual(row().calls, []);
  chat.toggle(chat.rows[1].id);
  assert.deepEqual(row().calls, [{ tool: "ls", target: "", reference: false, state: 1 }]);
});

test("an automatic retry continues the turn: one activity row, Working… again, its steps and time go on", () => {
  let clock = 0;
  const chat = new Chat(() => clock);
  chat.user("做个插件");
  chat.begin();
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "先看看。" });
  chat.apply({ kind: "tool_start", id: "a", name: "ls", target: "" });
  chat.apply({ kind: "tool_end", id: "a", name: "ls", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "529 overloaded" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity", "assistant", "error"], "the note stands in for the answer");
  chat.apply({ kind: "retry", attempt: 1, max: 2, error: "529 overloaded" });
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity", "notice"], "the note is a note again");
  assert.equal(activityLine(en, chat.rows[1]), "Working… · List ✓");
  assert.equal(chat.rows[1].state, 0);
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "tool_start", id: "b", name: "read", target: "a.md" });
  assert.equal(activityLine(en, chat.rows[1]), "Working… · Read a.md…");
  chat.apply({ kind: "tool_end", id: "b", name: "read", ok: true, summary: "" });
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "text", delta: "好了。" });
  chat.apply({ kind: "assistant_end", text: "好了。", stopReason: "stop", error: null });
  clock += 312_000;
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, shown(r), r.state]),
    [
      ["user", "做个插件", 1],
      ["activity", "用时 5 分 12 秒 · 2 个步骤", 1],
      ["notice", "连接失败，正在重试（1/2）…", 1],
      ["assistant", "好了。", 1],
    ],
  );
  assert.equal(activityLine(en, chat.rows[1]), "Worked for 5 min 12 s · 2 steps");

  // An error before any step: the row had gone; the retry brings it back after the message.
  chat.user("再来");
  chat.begin();
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "Connection error." });
  assert.deepEqual(chat.rows.slice(4).map((r) => r.kind), ["user", "error"]);
  chat.apply({ kind: "retry", attempt: 1, max: 2, error: "Connection error." });
  assert.deepEqual(chat.rows.slice(4).map((r) => [r.kind, shown(r)]), [["user", "再来"], ["activity", "处理中…"], ["notice", "连接失败，正在重试（1/2）…"]]);
  chat.apply({ kind: "text", delta: "好" });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.slice(4).map((r) => r.kind), ["user", "notice", "assistant"], "no step, no thinking: no row");
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
    { type: "message", message: { role: "user", content: "想想" } },
    { type: "message", message: { role: "assistant", content: [{ type: "thinking", thinking: "Hmm." }, { type: "text", text: "好" }], stopReason: "stop" } },
    { type: "message", message: { role: "user", content: "再来" } },
    { type: "message", message: { role: "assistant", content: [], stopReason: "error", errorMessage: "401 invalid x-api-key" } },
  ];
  const chat = Chat.fromJsonl(`${lines.map((l) => JSON.stringify(l)).join("\n")}\n{"type":"mess`);
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, shown(r)]),
    [
      ["user", "写文件"],
      // The record has no durations: the steps only.
      ["activity", "1 个步骤"],
      ["assistant", "写好了"],
      ["user", "想想"],
      ["assistant", "好"],
      ["user", "再来"],
      ["error", "API key 无效或没有权限，请在设置中检查。"],
    ],
  );
  assert.equal(activityLine(en, chat.rows[1]), "1 step");
  assert.equal(chat.rows[1].text, "");
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
  const chat = new Chat(() => 0);
  chat.user("做个插件");
  chat.begin();
  note(chat, "先检查。", "a");
  note(chat, "插件做好了，可以安装。", "b");
  // The model's last message is empty.
  chat.apply({ kind: "assistant_start" });
  chat.apply({ kind: "assistant_end", text: "", stopReason: "stop", error: null });
  chat.apply({ kind: "settled", aborted: false });
  assert.deepEqual(chat.rows.map((r) => [r.kind, shown(r)]), [
    ["user", "做个插件"],
    ["activity", "用时 1 秒 · 2 个步骤"],
    ["assistant", "插件做好了，可以安装。"],
  ]);

  // Stop after one note: the note is the answer.
  chat.user("再改改");
  chat.begin();
  note(chat, "改好了一半。", "c");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "aborted", error: null });
  assert.deepEqual(chat.rows.slice(3).map((r) => [r.kind, shown(r)]), [
    ["user", "再改改"],
    ["activity", "用时 1 秒 · 1 个步骤"],
    ["assistant", "改好了一半。"],
    ["notice", "已停止"],
  ]);

  // An error after a note: the note, then the error.
  chat.user("继续");
  chat.begin();
  note(chat, "正在测试。", "d");
  chat.apply({ kind: "assistant_end", text: "", stopReason: "error", error: "Connection error." });
  assert.deepEqual(chat.rows.slice(-3).map((r) => r.kind), ["activity", "assistant", "error"]);
  assert.equal(chat.rows.at(-2).text, "正在测试。");
  assert.equal(chat.rows.at(-3).state, 1, "the turn is over");
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

test("row order: the user's message, then the turn's activity, then the reply, live and from the record, for a fresh conversation and a follow-up", async () => {
  // Live: the model writes a note before its first tool call (shown as the
  // answer while it streams, then taken out), later the answer.
  const chat = new Chat();
  const turn = (prompt, note, answer) => {
    chat.user(prompt);
    chat.begin();
    chat.apply({ kind: "assistant_start" });
    chat.apply({ kind: "text", delta: note });
    assert.deepEqual(chat.rows.slice(-3).map((r) => r.kind), ["user", "activity", "assistant"], "the note streams below the message");
    chat.apply({ kind: "tool_start", id: `${prompt}-1`, name: "docs", target: "/" });
    chat.apply({ kind: "assistant_end", text: note, stopReason: "toolUse", error: null });
    chat.apply({ kind: "tool_end", id: `${prompt}-1`, name: "docs", ok: true, summary: "" });
    assert.deepEqual(chat.rows.slice(-2).map((r) => r.kind), ["user", "activity"], "the note is gone once its tool call starts");
    chat.apply({ kind: "assistant_start" });
    chat.apply({ kind: "text", delta: answer });
    chat.apply({ kind: "assistant_end", text: answer, stopReason: "stop", error: null });
    chat.apply({ kind: "settled", aborted: false });
  };
  turn("做个插件", "我先读文档。", "做好了。");
  turn("窗口打不开", "我看看。", "修好了。");
  assert.deepEqual(
    chat.rows.map((r) => [r.kind, r.text]),
    [["user", "做个插件"], ["activity", ""], ["assistant", "做好了。"], ["user", "窗口打不开"], ["activity", ""], ["assistant", "修好了。"]],
  );
  // What the window gets keeps that order (an unmade document shows as text).
  const docs = new MessageDocs(async (text) => [{ text }]);
  await docs.sync(chat.rows);
  assert.deepEqual(docs.props(chat.rows).map((r) => r.kind), ["user", "activity", "assistant", "user", "activity", "assistant"]);

  // The record, as pi writes it: the same two turns.
  const record = (role, content, extra = {}) => JSON.stringify({ type: "message", message: { role, content, ...extra } });
  const call = (id) => ({ type: "toolCall", id, name: "docs", arguments: { path: "/" } });
  const result = (id) => record("toolResult", [{ type: "text", text: "/ · docs" }], { toolCallId: id, isError: false });
  const jsonl = [
    JSON.stringify({ type: "session", id: "s" }),
    record("user", "做个插件"),
    record("assistant", [{ type: "thinking", thinking: "…" }, { type: "text", text: "我先读文档。" }, call("a")], { stopReason: "toolUse" }),
    result("a"),
    record("assistant", [{ type: "text", text: "做好了。" }], { stopReason: "stop" }),
    record("user", "窗口打不开"),
    record("assistant", [{ type: "text", text: "我看看。" }, call("b")], { stopReason: "toolUse" }),
    result("b"),
    record("assistant", [{ type: "text", text: "修好了。" }], { stopReason: "stop" }),
  ].join("\n");
  const replayed = Chat.fromJsonl(jsonl);
  assert.deepEqual(
    replayed.rows.map((r) => [r.kind, r.text]),
    [["user", "做个插件"], ["activity", ""], ["assistant", "做好了。"], ["user", "窗口打不开"], ["activity", ""], ["assistant", "修好了。"]],
  );
});

test("the last note becomes the answer also when thinking came after it; the thinking shows nowhere", () => {
  const chat = new Chat();
  chat.user("做个插件");
  chat.begin();
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
  assert.deepEqual(chat.rows.map((r) => r.kind), ["user", "activity", "assistant"]);
  assert.equal(chat.rows[2].text, "插件做好了。");
  assert.ok(!chat.rows.some((r) => /Plan it|nothing to add/.test(r.text)));
});
