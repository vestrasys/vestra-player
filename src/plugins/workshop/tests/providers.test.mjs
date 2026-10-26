// Providers: pi's key-based catalog, custom endpoints in OpenAI and Anthropic
// format (model list from the endpoint, or typed by hand), and the live
// status line while the agent works.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { CUSTOM, customList, listModels, loadProviders, pickListed } from "../providers.ts";
import { KEY, dataDirectory, server, startAi } from "./fixture.mjs";
import { listenFetchable } from "./mock-openai.mjs";

const catalog = async () => JSON.parse(await fs.readFile(new URL("../vendor/pi/models.json", import.meta.url), "utf8")).providers;

test("the catalog carries every key-based provider of pi, no login-only or cloud-credential ones", async () => {
  const providers = await catalog();
  const ids = providers.map((p) => p.id);
  for (const id of ["anthropic", "openai", "google", "openrouter", "deepseek", "mistral", "groq", "xai", "minimax", "moonshotai"])
    assert.ok(ids.includes(id), id);
  for (const id of ["openai-codex", "github-copilot", "amazon-bedrock", "google-vertex", "azure", "cloudflare-ai-gateway", "cloudflare-workers-ai"])
    assert.ok(!ids.includes(id), id);
  // User ruling: no coding-plan / token-plan / subscription entries.
  for (const id of ["kimi-coding", "zai", "zai-coding-cn", "qwen-token-plan", "qwen-token-plan-cn", "qwen-token-plan-individual", "xiaomi-token-plan-ams", "xiaomi-token-plan-cn", "xiaomi-token-plan-sgp", "opencode-go", "radius"])
    assert.ok(!ids.includes(id), id);
  for (const p of providers) assert.doesNotMatch(`${p.id} ${p.name}`, /plan|coding|subscription/i);
  for (const p of providers) assert.ok(p.models.length > 0 && p.name, p.id);

  const ai = await startAi();
  assert.deepEqual(ai.page.props.providers, [...providers.map((p) => p.name), "Custom · OpenAI format", "Custom · Anthropic format"]);
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
  assert.deepEqual(await listModels(customList(openai, mock.url), KEY), ["a-model", "b-model"]);
  assert.equal(mock.requests.at(-1).url, "/v1/models");
  assert.equal(mock.requests.at(-1).headers.authorization, `Bearer ${KEY}`);
  const root = mock.url.replace(/\/v1$/, "");
  assert.deepEqual(await listModels(customList(anthropic, root), KEY), ["a-model", "b-model"]);
  assert.equal(mock.requests.at(-1).url, "/v1/models");
  assert.equal(mock.requests.at(-1).headers["x-api-key"], KEY);
  assert.equal(mock.requests.at(-1).headers["anthropic-version"], "2023-06-01");

  const bare = await server(() => ({ text: ["x"] }), { models: null });
  await assert.rejects(listModels(customList(openai, bare.url), KEY), /404/);
});

test("Custom · Anthropic format: listed model, streamed reply and a tool call over the Anthropic wire format", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).content.some?.((part) => part.type === "tool_result")
      ? { text: ["写好了"] }
      : { text: ["好的"], toolCalls: [{ name: "write", args: { path: "a.md", content: "anthropic\n" } }] },
  );
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url.replace(/\/v1$/, ""), { format: "Anthropic" });
  assert.deepEqual(ai.page.props.models, ["mock-1", "mock-2"]);
  assert.equal(ai.modelLine(), "自定义 · Anthropic 格式 · mock-1");
  await ai.ask("写 a.md");
  assert.equal(ai.kinds(), "user,tools,assistant");
  // 「好的」 before the first tool call is an opening line, not shown (Chat.opening).
  assert.equal(ai.rows()[2].text, "写好了");
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
  const ai = await startAi({ locale: "zh-Hans" });
  await ai.configure(mock.url, { model: "typed-model" });
  assert.match(ai.message(), /没能获取模型列表|key 已保存/);
  assert.deepEqual(ai.page.props.models, []);
  assert.equal(ai.page.props["model-text"], "typed-model");
  await ai.ask("hi");
  assert.equal(ai.rows().at(-1).text, "你好");
  assert.equal(mock.requests.find((r) => r.method === "POST").body.model, "typed-model");
  assert.deepEqual(ai.host.secrets.get("key.custom-openai"), KEY);
  await ai.stop();
});

test("the status line is live: Thinking before the first event, each tool call, then the summary; it expands", async () => {
  const mock = await server((req) =>
    req.body.messages.at(-1).role === "tool"
      ? { text: ["完成"], delayMs: 1200 }
      : { toolCalls: [{ name: "write", args: { path: "notes.md", content: "x" } }], delayMs: 1200 },
  );
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("写 notes.md", { wait: false });
  const status = () => (ai.rows()[1]?.kind === "tools" ? ai.shown(1) : undefined);
  await ai.until(() => status() === "思考中…" && ai.rows()[1].state === 0, "thinking before the first event", 5_000);
  await ai.until(() => status() === "写入 notes.md ✓ · 思考中…", "the tool call, then thinking again", 5_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(status(), "1 个工具调用");
  assert.equal(ai.rows()[1].expanded, false);
  await ai.send("toggle", ai.rows()[1].id);
  assert.equal(ai.rows()[1].expanded, true);
  assert.deepEqual(ai.rows()[1].calls, [{ tool: "write", target: "notes.md", reference: false, state: 1 }], "the calls are sent while expanded");
  assert.equal(await ai.calls(1), "✓ 写入 notes.md");
  await ai.stop();
});

test("a reply without tool calls keeps no status line", async () => {
  const mock = await server(() => ({ text: ["只是回答"], delayMs: 300 }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("你好", { wait: false });
  await ai.until(() => ai.rows()[1]?.kind === "tools" && ai.shown(1) === "Thinking…", "thinking", 5_000);
  await ai.until(() => ai.props.busy === false, "end");
  assert.equal(ai.kinds(), "user,assistant");
  await ai.stop();
});

test("settings of the first slice move to Custom · OpenAI format with their Base URL, model and key", async () => {
  for (const old of ["vestra-compat", "openai-compatible"]) {
    const mock = await server(() => ({ text: ["迁移后可用"] }));
    const data = await dataDirectory();
    await fs.writeFile(
      path.join(data, "settings.json"),
      JSON.stringify({ provider: old, models: { [old]: "MiniMax-M3.1-Flash" }, baseUrl: mock.url, lastSession: null }),
    );
    const ai = await startAi({ data, secrets: { [`key.${old}`]: KEY } });
    await ai.until(() => ai.props.configured === true, "configured from the old settings");
    assert.equal(ai.modelLine(), "Custom · OpenAI format · MiniMax-M3.1-Flash");
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

test("pi's own User-Agent and headers go out unchanged: the worker never sets identity or header filters", async () => {
  // Kimi and OpenCode treat a changed User-Agent as a terms violation.
  for (const file of ["worker.ts", "tools.ts", "files.ts", "protocol.ts", "models.ts"]) {
    const source = await fs.readFile(new URL(`../worker/${file}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /userAgent|user-agent|identity|headers\s*:|filterHeaders|setGlobalDispatcher/i, file);
  }
  const mock = await server(() => ({ text: ["好"] }));
  const ai = await startAi({ compat: mock.url });
  await ai.ask("hi");
  const agent = mock.chats[0].headers["user-agent"];
  assert.ok(agent, "a User-Agent was sent");
  assert.doesNotMatch(agent, /vestra/i);
  await ai.stop();
});

test("a saved provider that is no longer offered: notice, nothing sent, settings and key kept until deleted", async () => {
  const mock = await server(() => ({ text: ["好"] }));
  const data = await dataDirectory();
  const saved = { provider: "zai", models: { zai: "glm-4.6" }, baseUrls: {}, lastSession: null, migrateKeys: false, savedKeys: { zai: true } };
  await fs.writeFile(path.join(data, "settings.json"), JSON.stringify(saved));
  const ai = await startAi({ data, secrets: { "key.zai": KEY }, locale: "zh-Hans" });
  const notice = "之前选择的服务商「Z.AI」已不再提供，请改用其他服务商或「自定义 · OpenAI/Anthropic 格式」。";
  await ai.until(() => ai.props.banner.code === "gone", "banner");
  assert.equal(ai.host.translate("The provider you chose before, “{}”, is no longer offered; use another provider or “Custom · OpenAI/Anthropic format”.", ai.props.banner.args), notice);
  assert.equal(ai.page.props.gone, "Z.AI");
  assert.equal(ai.keyState(), "旧服务商的 key 不会自动删除");
  assert.equal(ai.page.props.providers[ai.page.props["provider-index"]], "Z.AI（已不再提供）");
  assert.deepEqual(ai.page.props.providers.slice(-2), ["自定义 · OpenAI 格式", "自定义 · Anthropic 格式"]);
  assert.equal(ai.props.configured, false);
  await ai.send("send", "hi");
  assert.equal(mock.chats.length, 0);
  assert.equal(ai.rows().length, 0);
  // Nothing was changed or read: the old settings and the key stay.
  assert.equal(ai.host.calls.filter((c) => c.method === "secrets.get" && c.params.name === "key.zai").length, 0);
  assert.equal(ai.host.secrets.get("key.zai"), KEY);
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(data, "settings.json"), "utf8")).models, { zai: "glm-4.6" });
  ai.page.emit("delete-old-key");
  await ai.until(() => !ai.host.secrets.has("key.zai"), "old key deleted");
  // Picking a provider ends the notice.
  ai.page.emit("choose-provider", ai.page.props.providers.indexOf("自定义 · OpenAI 格式"));
  await ai.host.settle();
  assert.equal(ai.page.props.gone, "");
  await ai.configure(mock.url);
  await ai.ask("hi");
  assert.equal(ai.rows().at(-1).text, "好");
  await ai.stop();
});

test("the model shown is the model used: choosing a provider saves its default and the lists follow", async () => {
  const data = await dataDirectory();
  const ai = await startAi({ data });
  const providers = loadProviders(fileURLToPath(new URL("..", import.meta.url)));
  const minimax = providers.find((p) => p.id === "minimax");
  // The user picked something else in the list before (a one-way index would stop following).
  ai.page.emit("choose-provider", ai.page.props.providers.indexOf("OpenAI"));
  await ai.host.settle();
  ai.page.emit("choose-provider", ai.page.props.providers.indexOf("MiniMax"));
  await ai.host.settle();
  await ai.until(() => ai.page.props.models.length > 0, "models");
  const shown = ai.page.props.models[ai.page.props["model-index"]];
  assert.equal(shown, minimax.defaultModel);
  assert.equal(ai.page.props["model-text"], shown);
  assert.equal(ai.page.props.providers[ai.page.props["provider-index"]], "MiniMax");
  assert.equal(ai.page.props["can-list"], true);
  await ai.stop();
  const settings = JSON.parse(await fs.readFile(path.join(data, "settings.json"), "utf8"));
  assert.equal(settings.models.minimax, shown, "saved, so the request uses the same model");
});

test("listing styles of built-in providers, and which model to use after a listing", async () => {
  const mock = await server(() => ({ text: ["x"] }), { models: ["MiniMax-M3.1", "MiniMax-M3.1-Flash"] });
  assert.deepEqual(await listModels({ style: "anthropic", url: `${mock.url}/models` }, KEY), ["MiniMax-M3.1", "MiniMax-M3.1-Flash"]);
  assert.equal(mock.requests.at(-1).headers["x-api-key"], KEY);
  // Google: { models: [{ name: "models/<id>" }] }, key in x-goog-api-key.
  const http = await import("node:http");
  let header;
  const google = http.createServer((req, res) => {
    header = req.headers["x-goog-api-key"];
    res.end(JSON.stringify({ models: [{ name: "models/gemini-9" }, { name: "models/gemini-8" }] }));
  });
  await listenFetchable(google);
  assert.deepEqual(await listModels({ style: "google", url: `http://127.0.0.1:${google.address().port}/v1beta/models` }, KEY), ["gemini-8", "gemini-9"]);
  assert.equal(header, KEY);
  google.close();

  const providers = loadProviders(fileURLToPath(new URL("..", import.meta.url)));
  const minimax = providers.find((p) => p.id === "minimax");
  assert.equal(minimax.list.style, "anthropic");
  // A retired model (still in pi's catalog) is replaced by a listed one; a listed current one stays.
  assert.equal(pickListed(minimax, "MiniMax-M2.7", ["MiniMax-M3.1", "MiniMax-M3.1-Flash"]), "MiniMax-M3.1");
  assert.equal(pickListed(minimax, "MiniMax-M3.1-Flash", ["MiniMax-M3.1", "MiniMax-M3.1-Flash"]), "MiniMax-M3.1-Flash");
});

test("a listed model that pi's catalog does not know runs on the provider's endpoint", async () => {
  process.env.PI_OFFLINE = "1";
  process.env.PI_TELEMETRY = "0";
  process.env.PI_PACKAGE_DIR = fileURLToPath(new URL("../vendor/pi", import.meta.url));
  const sdk = await import("../vendor/pi/pi.mjs");
  const { resolveModel } = await import("../worker/models.ts");
  const runtime = await sdk.ModelRuntime.create({ credentials: new sdk.InMemoryCredentialStore(), modelsPath: null, refreshOnCreate: false });
  const known = runtime.getModels("minimax")[0];
  assert.equal(resolveModel(runtime, "minimax", known.id), runtime.getModel("minimax", known.id));
  const fresh = resolveModel(runtime, "minimax", "MiniMax-M9-Future");
  assert.equal(fresh.id, "MiniMax-M9-Future");
  assert.equal(fresh.provider, "minimax");
  assert.equal(fresh.baseUrl, known.baseUrl);
  assert.equal(resolveModel(runtime, "no-such-provider", "x"), undefined);
});

test("a model the provider no longer serves: a clear error with Open Settings and Retry", async () => {
  const mock = await server(() => ({ status: 404, error: "The model mock-1 is no longer available", code: "model_not_found" }));
  const ai = await startAi({ compat: mock.url, locale: "zh-Hans" });
  await ai.ask("hi");
  const error = ai.rows().at(-1);
  assert.equal(error.kind, "error");
  assert.deepEqual([error.code, error.args], ["model", ["mock-1"]]);
  assert.equal(ai.shown(-1), "模型「mock-1」不可用，请在设置中换一个模型。");
  assert.equal(error.settings, true);
  assert.match(error.detail, /no longer available/);
  assert.equal(ai.props["can-retry"], true);
  await ai.stop();
});
