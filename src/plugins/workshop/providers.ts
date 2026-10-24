// Services the user can bring a key for (BYOK). Model lists are pi's
// built-in catalog, captured in vendor/pi/models.json when pi was vendored.
import fs from "node:fs";
import path from "node:path";

/** 「OpenAI 兼容」: the worker registers the endpoint under this provider id. */
export const COMPAT_PROVIDER = "vestra-compat";

export interface Provider {
  id: string;
  label: string;
  /** Model ids to choose from; empty: typed by the user (OpenAI-compatible). */
  models: string[];
  defaultModel: string;
  /** A local server may need no key. */
  keyOptional: boolean;
}

const LABELS: [string, string, string[]][] = [
  ["anthropic", "Anthropic", ["claude-sonnet-4-5", "claude-sonnet-4-6"]],
  ["openai", "OpenAI", ["gpt-5-mini", "gpt-5"]],
  ["google", "Google", ["gemini-2.5-flash", "gemini-2.5-pro"]],
  ["openrouter", "OpenRouter", ["anthropic/claude-sonnet-4.5"]],
  ["deepseek", "DeepSeek", ["deepseek-flash"]],
];

export function loadProviders(packageDirectory: string): Provider[] {
  const catalog = JSON.parse(fs.readFileSync(path.join(packageDirectory, "vendor/pi/models.json"), "utf8")) as Record<string, string[]>;
  const providers = LABELS.map(([id, label, preferred]): Provider => {
    const models = catalog[id] ?? [];
    return { id, label, models, defaultModel: preferred.find((m) => models.includes(m)) ?? models[0] ?? "", keyOptional: false };
  });
  providers.push({ id: COMPAT_PROVIDER, label: "OpenAI 兼容（Ollama、LM Studio…）", models: [], defaultModel: "", keyOptional: true });
  return providers;
}

/** Keychain entry of a provider's key (one per provider). */
export const secretName = (provider: string) => `key.${provider}`;

/** What the settings page shows for a saved key: never the key itself. */
export function masked(key: string): string {
  return key ? `已保存 ••••${key.slice(-4)}` : "未保存";
}

export interface AiSettings {
  provider: string;
  /** The chosen model per provider. */
  models: Record<string, string>;
  /** OpenAI-compatible endpoint, e.g. http://localhost:11434/v1 */
  baseUrl: string;
  /** The session shown when the window opens again. */
  lastSession: string | null;
}

export const DEFAULT_SETTINGS: AiSettings = { provider: "anthropic", models: {}, baseUrl: "", lastSession: null };

export function validSettings(value: unknown): value is AiSettings {
  const v = value as AiSettings;
  return (
    !!v &&
    typeof v.provider === "string" &&
    !!v.models &&
    typeof v.models === "object" &&
    typeof v.baseUrl === "string" &&
    (v.lastSession === null || typeof v.lastSession === "string")
  );
}

/** A Base URL the worker can use: http(s), no credentials in it. */
export function checkBaseUrl(text: string): string {
  const url = new URL(text.trim());
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("Base URL 须为 http(s) 地址");
  if (url.username || url.password) throw new Error("Base URL 里不要写用户名或密码，key 请填在 API Key 中");
  return url.href.replace(/\/$/, "");
}
