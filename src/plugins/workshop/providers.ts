// Services the user can bring a key for (BYOK): every key-based provider in
// pi's catalog (vendor/pi/models.json, written when pi was vendored; see
// tests/vendor-pi.mjs for which are offered), plus two custom endpoints.
import fs from "node:fs";
import path from "node:path";

export interface Provider {
  id: string;
  label: string;
  /** Model ids from pi's catalog; empty for a custom endpoint (listed from it, or typed). */
  models: string[];
  defaultModel: string;
  /** Custom endpoints: the wire format pi speaks to them. */
  api: "openai-completions" | "anthropic-messages" | null;
}

/** Custom endpoints; the worker registers each under its id. */
export const CUSTOM: Provider[] = [
  { id: "custom-openai", label: "自定义 · OpenAI 格式", models: [], defaultModel: "", api: "openai-completions" },
  { id: "custom-anthropic", label: "自定义 · Anthropic 格式", models: [], defaultModel: "", api: "anthropic-messages" },
];

/** A sensible first choice where pi's catalog has it. */
const PREFERRED = ["claude-sonnet-4-5", "gpt-5-mini", "gemini-2.5-flash", "anthropic/claude-sonnet-4.5", "deepseek-flash"];

export function loadProviders(packageDirectory: string): Provider[] {
  const catalog = JSON.parse(fs.readFileSync(path.join(packageDirectory, "vendor/pi/models.json"), "utf8")) as {
    providers: { id: string; name: string; models: string[] }[];
  };
  const builtin = catalog.providers.map(
    (p): Provider => ({
      id: p.id,
      label: p.name,
      models: p.models,
      defaultModel: PREFERRED.find((m) => p.models.includes(m)) ?? p.models[0] ?? "",
      api: null,
    }),
  );
  return [...builtin, ...CUSTOM];
}

/** Keychain entry of a provider's key (one per provider id, custom entries included). */
export const secretName = (provider: string) => `key.${provider}`;

/** What the settings page shows for a saved key: never the key itself. */
export function masked(key: string): string {
  return key ? `已保存 ••••${key.slice(-4)}` : "未保存";
}

export interface AiSettings {
  provider: string;
  /** The chosen model per provider. */
  models: Record<string, string>;
  /** Base URL per custom endpoint. */
  baseUrls: Record<string, string>;
  /** The session shown when the window opens again. */
  lastSession: string | null;
  /** The first slice's key still has to move to custom-openai (done lazily, retried). */
  migrateKeys?: boolean;
}

export const DEFAULT_SETTINGS: AiSettings = { provider: "anthropic", models: {}, baseUrls: {}, lastSession: null };

const record = (v: unknown) => !!v && typeof v === "object" && Object.values(v).every((x) => typeof x === "string");
/** Ids the first slice used for its single OpenAI-compatible entry. */
export const LEGACY_COMPAT = ["vestra-compat", "openai-compatible"];

/** The current shape, or the first slice's (one `baseUrl`), which migrate() converts. */
export function validSettings(value: unknown): value is AiSettings {
  const v = value as AiSettings & { baseUrl?: unknown };
  return (
    !!v &&
    typeof v.provider === "string" &&
    record(v.models) &&
    (record(v.baseUrls) || typeof v.baseUrl === "string") &&
    (v.lastSession === null || v.lastSession === undefined || typeof v.lastSession === "string")
  );
}

/**
 * Settings of the first slice → current: its 「OpenAI 兼容」 entry becomes
 * 「自定义 · OpenAI 格式」 with the same Base URL and model. Null: already current.
 */
export function migrate(value: AiSettings & { baseUrl?: string }): AiSettings | null {
  if (value.baseUrls && !LEGACY_COMPAT.includes(value.provider) && value.baseUrl === undefined) return null;
  const models = { ...value.models };
  const baseUrls = { ...(value.baseUrls ?? {}) };
  for (const old of LEGACY_COMPAT) {
    if (models[old] && !models["custom-openai"]) models["custom-openai"] = models[old];
    delete models[old];
  }
  if (value.baseUrl && !baseUrls["custom-openai"]) baseUrls["custom-openai"] = value.baseUrl;
  return {
    provider: LEGACY_COMPAT.includes(value.provider) ? "custom-openai" : value.provider,
    models,
    baseUrls,
    lastSession: value.lastSession ?? null,
    migrateKeys: true,
  };
}

/** A Base URL the worker can use: http(s), no credentials in it, no trailing slash. */
export function checkBaseUrl(text: string): string {
  let url: URL;
  try {
    url = new URL(text.trim());
  } catch {
    throw new Error("Base URL 不是有效地址");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("Base URL 须为 http(s) 地址");
  if (url.username || url.password) throw new Error("Base URL 里不要写用户名或密码，key 请填在 API Key 中");
  return url.href.replace(/\/+$/, "");
}

/**
 * The models a custom endpoint lists: OpenAI format `GET {base}/models`,
 * Anthropic format `GET {base}/v1/models`. Throws when the endpoint has no
 * list or refuses; the user can then type the model id.
 */
export async function listModels(provider: Provider, baseUrl: string, key: string, timeoutMs = 10_000): Promise<string[]> {
  const anthropic = provider.api === "anthropic-messages";
  const url = anthropic ? `${baseUrl.replace(/\/v1$/, "")}/v1/models` : `${baseUrl}/models`;
  const headers: Record<string, string> = anthropic
    ? { "anthropic-version": "2023-06-01", ...(key ? { "x-api-key": key } : {}) }
    : key
      ? { authorization: `Bearer ${key}` }
      : {};
  const response = await fetch(url, { headers, signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`.trim());
  const body = (await response.json()) as { data?: { id?: unknown }[]; models?: { id?: unknown; name?: unknown }[] };
  // OpenAI and Anthropic: { data: [{ id }] }; Ollama's native list: { models: [{ name }] }.
  const items = body.data ?? body.models ?? [];
  const ids = items.map((m) => (typeof m.id === "string" ? m.id : typeof (m as { name?: unknown }).name === "string" ? String((m as { name: string }).name) : "")).filter(Boolean);
  if (!ids.length) throw new Error("没有列出任何模型");
  return [...new Set(ids)].sort();
}
