// Services the user can bring a key for (BYOK): every key-based provider in
// pi's catalog (vendor/pi/models.json, written when pi was vendored; see
// tests/vendor-pi.mjs for which are offered), plus two custom endpoints.
import fs from "node:fs";
import path from "node:path";

/** Where a provider lists its models: OpenAI, Anthropic or Google style. */
export interface ListSpec {
  style: "openai" | "anthropic" | "google";
  url: string;
}

export interface Provider {
  id: string;
  label: string;
  /** Model ids from pi's catalog; empty for a custom endpoint (listed from it, or typed). */
  models: string[];
  defaultModel: string;
  /** Custom endpoints: the wire format pi speaks to them. */
  api: "openai-completions" | "anthropic-messages" | null;
  /** Built-in providers whose model list we can ask for (pi's catalog can be out of date). */
  list?: ListSpec;
}

/** The list endpoint of a custom endpoint, from its Base URL. */
export function customList(provider: Provider, baseUrl: string): ListSpec {
  return provider.api === "anthropic-messages"
    ? { style: "anthropic", url: `${baseUrl.replace(/\/v1$/, "")}/v1/models` }
    : { style: "openai", url: `${baseUrl}/models` };
}

/**
 * The model to use after a successful listing: the current one if listed,
 * else a preferred or the catalog's default one if listed, else the first listed.
 */
export function pickListed(provider: Provider, current: string, listed: string[]): string {
  if (listed.includes(current)) return current;
  return PREFERRED.find((m) => listed.includes(m)) ?? (listed.includes(provider.defaultModel) ? provider.defaultModel : listed[0]);
}

/**
 * Custom endpoints; the worker registers each under its id. Their labels
 * are never shown as they are: the settings page's provider list names them
 * with texts (main.ts), the windows in their own words (labels.slint, by id).
 */
export const CUSTOM: Provider[] = [
  { id: "custom-openai", label: "Custom · OpenAI format", models: [], defaultModel: "", api: "openai-completions" },
  { id: "custom-anthropic", label: "Custom · Anthropic format", models: [], defaultModel: "", api: "anthropic-messages" },
];

/** A sensible first choice where pi's catalog has it. */
const PREFERRED = ["claude-sonnet-4-5", "gpt-5-mini", "gemini-2.5-flash", "anthropic/claude-sonnet-4.5", "deepseek-flash"];

export function loadProviders(packageDirectory: string): Provider[] {
  const catalog = JSON.parse(fs.readFileSync(path.join(packageDirectory, "vendor/pi/models.json"), "utf8")) as {
    providers: { id: string; name: string; models: string[]; list?: ListSpec }[];
  };
  const builtin = catalog.providers.map(
    (p): Provider => ({
      id: p.id,
      label: p.name,
      models: p.models,
      defaultModel: PREFERRED.find((m) => p.models.includes(m)) ?? p.models[0] ?? "",
      api: null,
      ...(p.list ? { list: p.list } : {}),
    }),
  );
  return [...builtin, ...CUSTOM];
}

/**
 * Names of providers that were offered once and are no longer (user ruling
 * 2026-10-24: no coding/token plans or subscriptions), for the notice.
 */
const REMOVED: Record<string, string> = {
  "kimi-coding": "Kimi For Coding",
  zai: "Z.AI",
  "zai-coding-cn": "Z.AI Coding CN",
  "qwen-token-plan": "Qwen Token Plan",
  "qwen-token-plan-cn": "Qwen Token Plan CN",
  "qwen-token-plan-individual": "Qwen Token Plan Individual",
  "xiaomi-token-plan-ams": "Xiaomi Token Plan AMS",
  "xiaomi-token-plan-cn": "Xiaomi Token Plan CN",
  "xiaomi-token-plan-sgp": "Xiaomi Token Plan SGP",
  "opencode-go": "OpenCode Go",
  radius: "Radius",
};

export const goneName = (id: string) => REMOVED[id] ?? id;


/** Keychain entry of a provider's key (one per provider id, custom entries included). */
export const secretName = (provider: string) => `key.${provider}`;

/**
 * A text a window shows: the key of an English template in the Slint file
 * (which has the words), its arguments (data) and a count for plurals.
 */
export type Say = {
  code: string;
  args: string[];
  n: number;
};

export const say = (code = "", args: (string | number)[] = [], n = 0): Say => ({ code, args: args.map(String), n });

/** What the settings page shows for a saved key: never the key itself (the last four characters). */
export function masked(key: string): Say {
  return key ? say("saved", [key.slice(-4)]) : say("not-saved");
}

export interface AiSettings {
  provider: string;
  /** The chosen model per provider. */
  models: Record<string, string>;
  /** Base URL per custom endpoint. */
  baseUrls: Record<string, string>;
  /** The session shown when the window opens again. */
  lastSession: string | null;
  /**
   * The first slice's key may still have to move to custom-openai (done
   * lazily, retried). Undefined: settings from 2.0.1/2.0.2, where a refused
   * move left no mark, so it is checked once more.
   */
  migrateKeys?: boolean;
  /**
   * Providers the user saved a key for (never the key). A custom endpoint
   * whose saved key cannot be read must not be called without it.
   */
  savedKeys?: Record<string, boolean>;
  /** The user's GitHub name: the id prefix of new plugins (unset: local.). */
  owner?: string;
  /** How much reasoning models think before they answer (unset: "medium"). */
  thinking?: Thinking;
  /**
   * Custom endpoints: whether the model reasons, by "<provider> <model>"; unset:
   * guessed from the model id (reasons()). pi's catalog knows it for its own providers.
   */
  reasoning?: Record<string, boolean>;
}

/** The thinking levels the settings offer, cheapest first; "medium" is the default (low did not complete plugin tasks in validation). */
export const THINKING = ["off", "low", "medium"] as const;
export type Thinking = (typeof THINKING)[number];
export const DEFAULT_THINKING: Thinking = "medium";

/**
 * Custom endpoint models that clearly reason (think before they answer) and
 * take the reasoning parameters: OpenAI o-series, GPT-5 (not its chat
 * variant) and gpt-oss, Claude 3.7 and 4+, Gemini 2.5+, DeepSeek R1 and
 * reasoner, MiniMax M1+, QwQ, Grok 3 mini, Magistral, and ids that say
 * "thinking" or "reasoning". Hybrid or unclear families (Qwen3 base and
 * instruct, DeepSeek V3.x, GLM, Kimi K2, Grok 3/4) are left out: when unsure,
 * no reasoning, which the settings' check box can change.
 */
const REASONING_IDS = [
  /(^|[/:])o[1-9](-(mini|pro|preview|high|low))*($|[-:@])/,
  /gpt-5(?!(\.\d+)?-chat)/,
  /gpt-oss/,
  /claude-3-7/,
  /claude-(opus|sonnet|haiku|fable)-([4-9]|\d{2})/,
  /gemini-(2\.5|[3-9])/,
  /deepseek-(r1|reasoner)/,
  /(^|[/:-])r1([-:]|$)/,
  /minimax-m\d/,
  /qwq/,
  /grok-3-mini/,
  /magistral/,
  /thinking|reasoning|reasoner/,
];
/** Ids that say they do not reason, whatever else they match. */
const NOT_REASONING = /non-?reasoning|no-?think|instruct/;
export const reasons = (model: string) => {
  const id = model.toLowerCase();
  return !NOT_REASONING.test(id) && REASONING_IDS.some((pattern) => pattern.test(id));
};
/** The key of a custom model in AiSettings.reasoning. */
export const reasoningKey = (provider: string, model: string) => `${provider} ${model}`;

export const DEFAULT_SETTINGS: AiSettings = { provider: "anthropic", models: {}, baseUrls: {}, lastSession: null, migrateKeys: false, savedKeys: {} };

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
    (v.lastSession === null || v.lastSession === undefined || typeof v.lastSession === "string") &&
    (v.savedKeys === undefined || (!!v.savedKeys && typeof v.savedKeys === "object")) &&
    (v.owner === undefined || typeof v.owner === "string")
    // thinking and reasoning are not checked here: an unknown level (a later
    // version's) or a damaged map must not throw away the provider, keys and
    // models; thinkingOf / reasoningOf fall back to the defaults instead.
  );
}

/** The saved thinking level, or the default for a missing or unknown one. */
export const thinkingOf = (settings: AiSettings): Thinking => (THINKING.includes(settings.thinking as Thinking) ? (settings.thinking as Thinking) : DEFAULT_THINKING);

/** The user's choice for a custom model ("<provider> <model>"), if a valid one was saved. */
export function reasoningOf(settings: AiSettings, key: string): boolean | undefined {
  const value = settings.reasoning && typeof settings.reasoning === "object" ? (settings.reasoning as Record<string, unknown>)[key] : undefined;
  return typeof value === "boolean" ? value : undefined;
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

/** Why a Base URL is refused: `code` is the settings page's message. */
export class BaseUrlError extends Error {
  readonly code: "url-invalid" | "url-scheme" | "url-credentials";
  constructor(code: BaseUrlError["code"], message: string) {
    super(message);
    this.code = code;
  }
}

/** A Base URL the worker can use: http(s), no credentials in it, no trailing slash. */
export function checkBaseUrl(text: string): string {
  let url: URL;
  try {
    url = new URL(text.trim());
  } catch {
    throw new BaseUrlError("url-invalid", "Base URL is not a valid address");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new BaseUrlError("url-scheme", "Base URL must be an http(s) address");
  if (url.username || url.password) throw new BaseUrlError("url-credentials", "Base URL must not contain a user name or password; put the key in API Key");
  return url.href.replace(/\/+$/, "");
}

/** An endpoint answered with an empty model list. */
export class NoModelsError extends Error {}

/**
 * The models a custom endpoint lists: OpenAI format `GET {base}/models`,
 * Anthropic format `GET {base}/v1/models`. Throws when the endpoint has no
 * list or refuses; the user can then type the model id.
 */
export async function listModels(spec: ListSpec, key: string, timeoutMs = 10_000): Promise<string[]> {
  const headers: Record<string, string> =
    spec.style === "anthropic"
      ? { "anthropic-version": "2023-06-01", ...(key ? { "x-api-key": key } : {}) }
      : spec.style === "google"
        ? key
          ? { "x-goog-api-key": key }
          : {}
        : key
          ? { authorization: `Bearer ${key}` }
          : {};
  const response = await fetch(spec.url, { headers, signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`.trim());
  const body = (await response.json()) as { data?: { id?: unknown }[]; models?: { id?: unknown; name?: unknown }[] };
  // OpenAI and Anthropic: { data: [{ id }] }; Google and Ollama: { models: [{ name }] }
  // (Google's names are "models/<id>").
  const items = body.data ?? body.models ?? [];
  const ids = items
    .map((m) => (typeof m.id === "string" ? m.id : typeof (m as { name?: unknown }).name === "string" ? String((m as { name: string }).name) : ""))
    .map((id) => id.replace(/^models\//, ""))
    .filter(Boolean);
  if (!ids.length) throw new NoModelsError("No models listed");
  return [...new Set(ids)].sort();
}
