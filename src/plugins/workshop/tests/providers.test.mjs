// Providers: pi's key-based catalog, custom endpoints in OpenAI and Anthropic
// format (model list from the endpoint, or typed by hand), and the live
// status line while the agent works.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { CUSTOM, listModels } from "../providers.ts";
import { KEY, dataDirectory, server, startAi } from "./fixture.mjs";

const catalog = async () => JSON.parse(await fs.readFile(new URL("../vendor/pi/models.json", import.meta.url), "utf8")).providers;

test("the catalog carries every key-based provider of pi, no login-only or cloud-credential ones", async () => {
  const providers = await catalog();
  const ids = providers.map((p) => p.id);
  for (const id of ["anthropic", "openai", "google", "openrouter", "deepseek", "mistral", "groq", "xai", "minimax", "moonshotai", "zai", "qwen-token-plan"])
    assert.ok(ids.includes(id), id);
  for (const id of ["openai-codex", "github-copilot", "amazon-bedrock", "google-vertex", "azure", "cloudflare-ai-gateway", "cloudflare-workers-ai"])
    assert.ok(!ids.includes(id), id);
  for (const p of providers) assert.ok(p.models.length > 0 && p.name, p.id);

  const ai = await startAi();
  assert.deepEqual(ai.page.props.providers, [...providers.map((p) => p.name), "自定义 · OpenAI 格式", "自定义 · Anthropic 格式"]);
  // Choosing a provider fills the model list from the catalog.
  const mistral = providers.findIndex((p) => p.id === "mistral");
  ai.page.emit("choose-provider", mistral);
  await ai.host.settle();
  assert.deepEqual(ai.page.props.models, providers[mistral].models);
  assert.equal(ai.page.props.custom, false);
  await ai.stop();
});

test("custom endpoints list their models: OpenAI GET /models, Anthropic GET /v1/models", async () => {
  const mock = await server(() => ({ text: ["x"] }), { models: ["b-model", "a-model"] });
  const [openai, anthropic] = CUSTOM;
  assert.deepEqual(await listModels(openai, mock.url, KEY), ["a-model", "b-model"]);
  assert.equal(mock.requests.at(-1).url, "/v1/models");
  assert.equal(mock.requests.at(-1).headers.authorization, `Bearer ${KEY}`);
  const root = mock.url.replace(/\/v1$/, "");
  assert.deepEqual(await listModels(anthropic, root, KEY), ["a-model", "b-model"]);
  assert.equal(mock.requests.at(-1).url, "/v1/models");
  assert.equal(mock.requests.at(-1).headers["x-api-key"], KEY);
  assert.equal(mock.requests.at(-1).headers["anthropic-version"], "2023-06-01");

  const bare = await server(() => ({ text: ["x"] }), { models: null });
  await assert.rejects(listModels(openai, bare.url, KEY), /404/);
});

test("「自定义 · Anthropic 格式」: listed model, streamed reply and a tool call over the Anthropic wire format", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).content.some?.((part) => part.type === "tool_result")
      ? { text: ["写好了"] }
      : { text: ["好的"], toolCalls: [{ name: "write", args: { path: "a.md", content: "anthropic\n" } }] },
  );
  const ai = await startAi();
  await ai.configure(mock.url.replace(/\/v1$/, ""), { format: "Anthropic" });
  assert.deepEqual(ai.page.props.models, ["mock-1", "mock-2"]);
  assert.equal(ai.props["model-text"], "自定义 · Anthropic 格式 · mock-1");
  await ai.ask("写 a.md");
  assert.equal(ai.kinds(), "user,tools,assistant");
  assert.equal(ai.rows()[2].text, "好的\n\n写好了");
  const chat = mock.requests.filter((r) => r.method === "POST");
  assert.equal(new URL(chat[0].url, "http://x").pathname, "/v1/messages");
  assert.equal(chat[0].headers["x-api-key"], KEY);
  assert.equal(chat[0].body.model, "mock-1");
  const [sid] = await fs.readdir(path.join(ai.plugin.dataDirectory, "sessions"));
  assert.equal(await fs.readFile(path.join(ai.plugin.dataDirectory, "sessions", sid, "work/a.md"), "utf8"), "anthropic\n");
  await ai.stop();
});

test("an endpoint without a model list: say so, and a typed model id works", async () => {
  const mock = await server(() => ({ text: ["你好"] }), { models: null });
  const ai = await startAi();
  await ai.configure(mock.url, { model: "typed-model" });
  assert.match(ai.page.props.message, /没能获取模型列表|key 已保存/);
  assert.deepEqual(ai.page.props.models, []);
  assert.equal(ai.page.props["model-text"], "typed-model");
  await ai.ask("hi");
  assert.equal(ai.rows().at(-1).text, "你好");
  assert.equal(mock.requests.find((r) => r.method === "POST").body.model, "typed-model");
  assert.deepEqual(ai.host.secrets.get("key.custom-openai"), KEY);
  await ai.stop();
});

test("the status line is live: 思考中 before the first event, each tool call, then the summary; it expands", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).role === "tool"
      ? { text: ["完成"], delayMs: 1200 }
      : { toolCalls: [{ name: "write", args: { path: "notes.md", content: "x" } }], delayMs: 1200 },
  );
  const ai = await startAi({ compat: mock.url });
  await ai.ask("写 notes.md", { wait: false });
  const status = () => ai.rows()[1]?.text;
  await ai.until(() => status() === "思考中…" && ai.rows()[1].state === 0, "thinking before the first event", 5_000);
  await ai.until(() => status() === "写入 notes.md ✓ · 思考中…", "the tool call, then thinking again", 5_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(status(), "1 个工具调用");
  assert.equal(ai.rows()[1].expanded, false);
  await ai.send("toggle", ai.rows()[1].id);
  assert.equal(ai.rows()[1].expanded, true);
  assert.equal(ai.rows()[1].detail, "✓ 写入 notes.md");
  await ai.stop();
});

test("a reply without tool calls keeps no status line", async () => {
  const mock = await server(() => ({ text: ["只是回答"], delayMs: 300 }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("你好", { wait: false });
  await ai.until(() => ai.rows()[1]?.text === "思考中…", "thinking", 5_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(ai.kinds(), "user,assistant");
  await ai.stop();
});

test("settings of the first slice move to 「自定义 · OpenAI 格式」 with their Base URL, model and key", async () => {
  for (const old of ["vestra-compat", "openai-compatible"]) {
    const mock = await server(() => ({ text: ["迁移后可用"] }));
    const data = await dataDirectory();
    await fs.writeFile(
      path.join(data, "settings.json"),
      JSON.stringify({ provider: old, models: { [old]: "MiniMax-M3.1-Flash" }, baseUrl: mock.url, lastSession: null }),
    );
    const ai = await startAi({ data, secrets: { [`key.${old}`]: KEY } });
    await ai.until(() => ai.props.configured === true, "configured from the old settings");
    assert.equal(ai.props["model-text"], "自定义 · OpenAI 格式 · MiniMax-M3.1-Flash");
    assert.equal(ai.page.props["base-url"], mock.url);
    assert.equal(ai.host.secrets.get("key.custom-openai"), KEY);
    assert.equal(ai.host.secrets.has(`key.${old}`), false);
    await ai.ask("hi");
    assert.equal(ai.rows().at(-1).text, "迁移后可用");
    assert.equal(mock.requests.find((r) => r.method === "POST").headers.authorization, `Bearer ${KEY}`);
    await ai.stop();
    const saved = JSON.parse(await fs.readFile(path.join(data, "settings.json"), "utf8"));
    assert.equal(saved.provider, "custom-openai");
    assert.equal(saved.baseUrl, undefined);
  }
});
