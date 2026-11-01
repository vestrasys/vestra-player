// Reasoning models: thinking is on ("low") where the model reasons, comes
// back as thinking (a collapsed row), not as the answer, and the settings
// choose the level and, for a custom endpoint, whether its model reasons.
import assert from "node:assert/strict";
import test from "node:test";
import { reasons } from "../providers.ts";
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

test("a custom Anthropic-format reasoning model (MiniMax-M3): thinking low with room for the answer, shown collapsed", async () => {
  const mock = await server(thinkThenAnswer, { models: ["MiniMax-M3"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { format: "Anthropic" });
  assert.equal(ai.page.props["thinking-index"], 1, "low by default");
  assert.equal(ai.page.props.reasons, true, "MiniMax-M3 reasons (guessed from its id)");
  await ai.ask("看看文件夹");

  const body = mock.chats[0].body;
  assert.equal(body.thinking?.type, "enabled");
  assert.equal(body.thinking.budget_tokens, 2048);
  assert.ok(body.max_tokens > body.thinking.budget_tokens + 8000, `max_tokens ${body.max_tokens} leaves room after the thinking budget`);

  assert.equal(ai.kinds(), "user,thought,tools,assistant");
  assert.equal(ai.rows()[3].text, "文件夹是空的。", "the answer has no thinking in it");
  assert.match(ai.shown(1), /^已思考 \d+ 秒$/);
  await ai.send("toggle", ai.rows()[1].id);
  assert.equal(ai.rows()[1].text, "The user wants the folder listed.\n\n我先看看文件夹。\n\nThe folder is empty.");
  // The thinking went back to the model with its signature (Anthropic requires it with tool use).
  const replay = mock.chats[1].body.messages.find((m) => m.role === "assistant");
  assert.deepEqual(replay.content[0], { type: "thinking", thinking: "The user wants the folder listed.", signature: "sig-test" });
  await ai.stop();
});

test("thinking off in the settings, or a model marked as not reasoning, sends no thinking", async () => {
  const mock = await server(() => ({ text: ["好"] }), { models: ["MiniMax-M3"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { format: "Anthropic" });
  ai.page.emit("choose-thinking", 0);
  await ai.host.settle();
  await ai.ask("一");
  assert.deepEqual(mock.chats.at(-1).body.thinking, { type: "disabled" });
  assert.equal(mock.chats.at(-1).body.max_tokens, 16384);

  ai.page.emit("choose-thinking", 2);
  await ai.send("new-chat");
  await ai.ask("二");
  assert.equal(mock.chats.at(-1).body.thinking.budget_tokens, 8192, "medium");

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
  const mock = await server(() => ({ thinking: ["Short question."], text: ["你好！"] }), { models: ["qwen3:8b"] });
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url);
  await ai.ask("你好");
  assert.equal(ai.kinds(), "user,thought,assistant");
  assert.equal(ai.rows()[2].text, "你好！");
  await ai.stop();
});

test("which custom models reason, by id", () => {
  for (const id of ["MiniMax-M3", "minimax-m3.1-flash", "deepseek-reasoner", "qwen3:8b", "o3-mini", "gpt-5-mini", "claude-sonnet-4-5", "gemini-2.5-flash", "glm-4.6", "kimi-k2-thinking"])
    assert.equal(reasons(id), true, id);
  for (const id of ["gpt-4o", "deepseek-chat", "llama3.1:8b", "claude-3-5-sonnet", "gemini-2.0-flash", "mock-1", "gemma3"]) assert.equal(reasons(id), false, id);
});
