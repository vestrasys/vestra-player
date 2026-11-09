// Reasoning models: thinking is on ("medium" by default) where the model reasons, comes
// back as thinking (not shown: it only counts as the turn's work), not as the answer, and the settings
// choose the level and, for a custom endpoint, whether its model reasons.
import assert from "node:assert/strict";
import test from "node:test";
import { DEFAULT_SETTINGS, reasoningOf, reasons, thinkingOf, validSettings } from "../providers.ts";
import { server, startAi } from "./fixture.mjs";

/** Thinks, writes a note and calls ls; after the tool result, thinks again and answers. */
const thinkThenAnswer = (req) => {
  const messages = req.body.messages;
  const last = messages.at(-1);
  const afterTool = last.role === "tool" || (Array.isArray(last.content) && last.content.some((p) => p.type === "tool_result"));
  return afterTool
    ? { thinking: ["The folder is empty."], text: ["文件夹是空的。"] }
    : { thinking: ["The user wants ", "the folder listed."], text: ["我先看看文件夹。"], toolCalls: [{ name: "ls", args: { path: "." } }] };
};

test("a custom Anthropic-format reasoning model (MiniMax-M3): thinking medium by default with room for the answer, not shown", async () => {
  const mock = await server(thinkThenAnswer, { models: ["MiniMax-M3"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { format: "Anthropic" });
  assert.equal(ai.page.props["thinking-index"], 2, "medium by default");
  assert.equal(ai.page.props.reasons, true, "MiniMax-M3 reasons (guessed from its id)");
  await ai.ask("看看文件夹");

  const body = mock.chats[0].body;
  assert.equal(body.thinking?.type, "enabled");
  assert.equal(body.thinking.budget_tokens, 8192);
  assert.ok(body.max_tokens > body.thinking.budget_tokens + 8000, `max_tokens ${body.max_tokens} leaves room after the thinking budget`);

  assert.equal(ai.kinds(), "user,activity,assistant");
  assert.equal(ai.rows()[2].text, "文件夹是空的。", "the answer has no thinking in it");
  assert.match(ai.shown(1), /^用时 \d+ 秒 · 1 个步骤$/);
  await ai.send("toggle", ai.rows()[1].id);
  assert.equal(ai.rows()[1].text, "", "no thinking sent to the window");
  assert.equal(await ai.calls(1), "✓ 列出 .");
  // The thinking went back to the model with its signature (Anthropic requires it with tool use).
  const replay = mock.chats[1].body.messages.find((m) => m.role === "assistant");
  assert.deepEqual(replay.content[0], { type: "thinking", thinking: "The user wants the folder listed.", signature: "sig-test" });
  await ai.stop();
});

test("thinking off or low in the settings, or a model marked as not reasoning: no thinking or the smaller budget", async () => {
  const mock = await server(() => ({ text: ["好"] }), { models: ["MiniMax-M3"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { format: "Anthropic" });
  ai.page.emit("choose-thinking", 0);
  await ai.host.settle();
  await ai.ask("一");
  assert.deepEqual(mock.chats.at(-1).body.thinking, { type: "disabled" });
  assert.equal(mock.chats.at(-1).body.max_tokens, 16384);

  ai.page.emit("choose-thinking", 1);
  await ai.send("new-chat");
  await ai.ask("二");
  assert.equal(mock.chats.at(-1).body.thinking.budget_tokens, 2048, "low");

  ai.page.emit("set-reasons", false);
  await ai.host.settle();
  assert.equal(ai.page.props.reasons, false);
  await ai.send("new-chat");
  await ai.ask("三");
  assert.equal(mock.chats.at(-1).body.thinking, undefined);
  assert.equal(mock.chats.at(-1).body.max_tokens, 8192);
  await ai.stop();
});

test("an OpenAI-format endpoint's reasoning_content arrives as thinking, not as the answer", async () => {
  const mock = await server(() => ({ thinking: ["Short question."], text: ["你好！"] }), { models: ["qwq-32b"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url);
  await ai.ask("你好");
  assert.equal(ai.kinds(), "user,activity,assistant");
  assert.match(ai.shown(1), /^用时 \d+ 秒$/, "a turn that thought says how long it worked");
  assert.equal(ai.rows()[2].text, "你好！");
  await ai.stop();
});

test("which custom models reason, by id: only clear reasoning models; when unsure, not", () => {
  const yes = ["MiniMax-M3", "minimax-m3.1-flash", "deepseek-reasoner", "deepseek-r1", "o3-mini", "gpt-5-mini", "gpt-oss-120b", "claude-sonnet-4-5", "gemini-2.5-flash"];
  const thinkingVariants = ["qwen3-235b-a22b-thinking-2507", "kimi-k2-thinking", "grok-3-mini", "grok-4-fast-reasoning", "qwq-32b", "magistral-medium"];
  for (const id of [...yes, ...thinkingVariants]) assert.equal(reasons(id), true, id);
  const no = ["gpt-4o", "gpt-5-chat-latest", "deepseek-chat", "deepseek-v3.1", "llama3.1:8b", "claude-3-5-sonnet", "gemini-2.0-flash", "mock-1", "gemma3"];
  const unsure = ["qwen3:8b", "qwen3-coder-480b", "Qwen3-235B-A22B-Instruct-2507", "kimi-k2-instruct", "grok-3", "grok-4", "grok-4-fast-non-reasoning", "glm-4.6"];
  for (const id of [...no, ...unsure]) assert.equal(reasons(id), false, id);
});

test("an unknown thinking level or a damaged reasoning map keeps the other settings", () => {
  const saved = { ...DEFAULT_SETTINGS, provider: "custom-openai", models: { "custom-openai": "m" }, thinking: "high", reasoning: { "custom-openai m": "yes" } };
  assert.equal(validSettings(saved), true);
  assert.equal(thinkingOf(saved), "medium", "the default");
  assert.equal(thinkingOf({ ...saved, thinking: "low" }), "low", "a saved choice stays as saved");
  assert.equal(thinkingOf({ ...saved, thinking: "off" }), "off");
  assert.equal(thinkingOf(DEFAULT_SETTINGS), "medium", "unset: medium");
  assert.equal(reasoningOf(saved, "custom-openai m"), undefined);
  assert.equal(reasoningOf({ ...saved, reasoning: { "custom-openai m": false } }, "custom-openai m"), false);
});

test("a thinking change while the agent generates applies to the next message (a new worker continues the conversation)", async () => {
  let first = true;
  const mock = await server(() => {
    const answer = first ? { text: ["一"], delayMs: 1500 } : { text: ["二"] };
    first = false;
    return answer;
  }, { models: ["MiniMax-M3"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { format: "Anthropic" });
  await ai.ask("一", { wait: false });
  await ai.until(() => ai.props.busy === true && mock.chats.length === 1, "generating");
  ai.page.emit("choose-thinking", 0);
  await ai.until(() => ai.props.busy === false, "first reply");
  assert.equal(mock.chats[0].body.thinking.type, "enabled");
  await ai.ask("二");
  const second = mock.chats.at(-1).body;
  assert.deepEqual(second.thinking, { type: "disabled" });
  assert.deepEqual(second.messages.map((m) => m.role), ["user", "assistant", "user"], "the same conversation");
  await ai.stop();
});
