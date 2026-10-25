// Subtitle services over HTTPS with `fetch`: OpenSubtitles.com REST API (default)
// and assrt.net (optional). Every request needs the user's own key or token;
// nothing is sent without one. Each request gives up after 15 seconds.
import { extname } from "node:path";
import { STATUS, line, raw } from "./messages.ts";
import type { Line, StatusId } from "./messages.ts";

export type ServiceId = "opensubtitles" | "assrt";
export const SERVICE_IDS: ServiceId[] = ["opensubtitles", "assrt"];
export const USER_AGENT = "Vestra Player v0.1.0";
export const REQUEST_TIMEOUT = 15_000;

export type Credentials = { apiKey: string; username: string; password: string; token: string };
export type Query = { title: string; hash: string | null; languages: string[]; page: number };
export type Result = {
  key: string;
  service: ServiceId;
  /** Language code as the service gives it, e.g. "zh-CN". */
  language: string;
  release: string;
  format: string;
  fps: number | null;
  downloads: number | null;
  hashMatch: boolean;
  machine: boolean;
  hearingImpaired: boolean;
  url: string | null;
  /** What `download` needs: an OpenSubtitles file id or an assrt subtitle id. */
  ref: number;
  fileName: string;
};
export type Download = { files: { name: string; bytes: Buffer }[]; remaining: number | null };

/** A failure the user can act on; `status` is the line to show (messages.ts). */
export class ServiceError extends Error {
  readonly kind: "auth" | "quota" | "network" | "invalid" | "aborted";
  readonly status: Line;
  readonly reset: string | null;
  constructor(kind: ServiceError["kind"], status: StatusId | Line, args: (string | number)[] = [], reset: string | null = null) {
    const shown = typeof status === "string" ? line(status, args) : status;
    super(shown.id === "raw" ? String(shown.args[0]) : shown.id ? STATUS[shown.id] : "");
    this.kind = kind;
    this.status = shown;
    this.reset = reset;
  }
}

/**
 * Language names for track titles and the download window's rows (translated
 * by the player, model.ts `trackTitle` and `row`).
 */
const LANGUAGE_LABELS = {
  "zh-cn": "Simplified Chinese", "zh-tw": "Traditional Chinese", zh: "Chinese", en: "English", ja: "Japanese",
  ko: "Korean", fr: "French", de: "German", es: "Spanish", it: "Italian", ru: "Russian", pt: "Portuguese",
  "pt-br": "Brazilian Portuguese", ar: "Arabic", th: "Thai", vi: "Vietnamese", "zh-ca": "Cantonese",
} as const;
type LanguageCode = keyof typeof LANGUAGE_LABELS;
export const LANGUAGE_CODES = Object.keys(LANGUAGE_LABELS);
/** The English name of a known language code (the source text), else undefined. */
export const languageLabel = (code: string): (typeof LANGUAGE_LABELS)[LanguageCode] | undefined =>
  LANGUAGE_LABELS[code.toLowerCase() as LanguageCode];

async function request(url: string, init: RequestInit, signal: AbortSignal, service: ServiceId) {
  const timeout = AbortSignal.timeout(REQUEST_TIMEOUT);
  try {
    return await fetch(url, { ...init, signal: AbortSignal.any([signal, timeout]) });
  } catch {
    if (signal.aborted) throw new ServiceError("aborted", "aborted");
    throw new ServiceError("network", `connect.${service}` as const);
  }
}
async function json(response: Response, service: ServiceId): Promise<Record<string, unknown>> {
  try {
    const value = await response.json();
    return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  } catch {
    throw new ServiceError("network", `unreadable.${service}` as const);
  }
}
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);
const str = (value: unknown) => (typeof value === "string" ? value : typeof value === "number" ? String(value) : "");
const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : null);
const formatOf = (name: string) => extname(name).slice(1).toLowerCase() || "srt";

export interface Service {
  readonly id: ServiceId;
  search(query: Query, signal: AbortSignal): Promise<{ results: Result[]; more: boolean }>;
  download(result: Result, signal: AbortSignal): Promise<Download>;
  /** A short line: whether the key works, and the remaining downloads when known. */
  test(signal: AbortSignal): Promise<Line>;
}

// ------------------------------------------------------------------ OpenSubtitles
const OS_BASE = "https://api.opensubtitles.com/api/v1";

export class OpenSubtitles implements Service {
  readonly id = "opensubtitles" as const;
  readonly #credentials: () => Credentials;
  /** Login token (memory only) and the server it was issued for. */
  #session: { token: string; base: string; user: string } | null = null;
  constructor(credentials: () => Credentials) {
    this.#credentials = credentials;
  }
  #headers(token?: string) {
    const headers: Record<string, string> = {
      "Api-Key": this.#credentials().apiKey,
      "User-Agent": USER_AGENT,
      Accept: "application/json",
      "Content-Type": "application/json",
    };
    if (token) headers.Authorization = `Bearer ${token}`;
    return headers;
  }
  async #check(response: Response) {
    if (response.status === 401 || response.status === 403) {
      this.#session = null;
      throw new ServiceError("auth", "osBadKey");
    }
    if (response.status === 406 || response.status === 429) {
      const body = await json(response, this.id).catch(() => ({}) as Record<string, unknown>);
      if (response.status === 406 || num(body.remaining) === 0)
        throw new ServiceError("quota", "osQuota", [], str(body.reset_time) || null);
      throw new ServiceError("network", "osTooMany");
    }
    if (!response.ok) throw new ServiceError("network", "osError", [response.status]);
  }
  /** Log in when a user name is set; a failed login is reported and searches go on anonymously. */
  async #login(signal: AbortSignal) {
    const { username, password } = this.#credentials();
    if (!username || !password) return null;
    if (this.#session?.user === username) return this.#session;
    const response = await request(`${OS_BASE}/login`, {
      method: "POST", headers: this.#headers(), body: JSON.stringify({ username, password }),
    }, signal, this.id);
    if (response.status === 401) throw new ServiceError("auth", "osBadLogin");
    await this.#check(response);
    const body = await json(response, this.id);
    const token = str(body.token);
    if (!token) throw new ServiceError("auth", "osLoginFailed");
    const host = str(body.base_url).replace(/^https?:\/\//, "").replace(/\/.*$/, "");
    this.#session = { token, base: host ? `https://${host}/api/v1` : OS_BASE, user: username };
    return this.#session;
  }
  async search(query: Query, signal: AbortSignal) {
    // Parameters sorted and lower-case, as the API asks (otherwise it redirects).
    const params: [string, string][] = [];
    if (query.languages.length) params.push(["languages", query.languages.map((l) => l.toLowerCase()).sort().join(",")]);
    if (query.hash) params.push(["moviehash", query.hash.toLowerCase()]);
    if (query.page > 1) params.push(["page", String(query.page)]);
    if (query.title) params.push(["query", query.title.toLowerCase()]);
    params.sort(([a], [b]) => a.localeCompare(b));
    const url = `${OS_BASE}/subtitles?${new URLSearchParams(params).toString()}`;
    const response = await request(url, { headers: this.#headers() }, signal, this.id);
    await this.#check(response);
    const body = await json(response, this.id);
    const results: Result[] = [];
    for (const item of list(body.data)) {
      const a = record(record(item).attributes);
      const file = record(list(a.files)[0]);
      const fileId = num(file.file_id);
      if (fileId === null) continue;
      const fileName = str(file.file_name) || `${str(a.release) || "subtitle"}.srt`;
      results.push({
        key: `opensubtitles:${fileId}`,
        service: this.id,
        language: str(a.language) || "?",
        release: str(a.release) || str(record(a.feature_details).title) || fileName,
        format: formatOf(fileName),
        fps: num(a.fps) || null,
        downloads: num(a.download_count),
        hashMatch: a.moviehash_match === true,
        machine: a.machine_translated === true || a.ai_translated === true,
        hearingImpaired: a.hearing_impaired === true,
        url: str(a.url) || null,
        ref: fileId,
        fileName,
      });
    }
    const page = num(body.page) ?? query.page, pages = num(body.total_pages) ?? page;
    return { results, more: page < pages };
  }
  async download(result: Result, signal: AbortSignal): Promise<Download> {
    const session = await this.#login(signal).catch((e) => {
      if (e instanceof ServiceError && e.kind === "aborted") throw e;
      return null; // download anonymously
    });
    const response = await request(`${session?.base ?? OS_BASE}/download`, {
      method: "POST", headers: this.#headers(session?.token), body: JSON.stringify({ file_id: result.ref }),
    }, signal, this.id);
    await this.#check(response);
    const body = await json(response, this.id);
    const link = str(body.link);
    if (!link) throw new ServiceError("network", str(body.message) ? raw(str(body.message)) : line("osNoLink"));
    const file = await request(link, { headers: { "User-Agent": USER_AGENT } }, signal, this.id);
    if (!file.ok) throw new ServiceError("network", "downloadFailed", [file.status]);
    return {
      files: [{ name: str(body.file_name) || result.fileName, bytes: Buffer.from(await file.arrayBuffer()) }],
      remaining: num(body.remaining),
    };
  }
  async test(signal: AbortSignal) {
    if (!this.#credentials().apiKey) return line("keyFirst");
    const session = await this.#login(signal);
    if (session) {
      const response = await request(`${session.base}/infos/user`, { headers: this.#headers(session.token) }, signal, this.id);
      await this.#check(response);
      const remaining = num(record((await json(response, this.id)).data).remaining_downloads);
      return remaining === null ? line("keyAccountValid") : line("keyAccountRemaining", [remaining]);
    }
    const response = await request(`${OS_BASE}/infos/formats`, { headers: this.#headers() }, signal, this.id);
    await this.#check(response);
    return line("keyValid");
  }
}

// ------------------------------------------------------------------ assrt.net
const ASSRT_BASE = "https://api.assrt.net/v1";
const ASSRT_LANGUAGES: [string, string][] = [
  ["langchs", "zh-CN"], ["langcht", "zh-TW"], ["langdou", "zh-CN"], ["langeng", "en"], ["langjap", "ja"], ["langkor", "ko"],
];

export class Assrt implements Service {
  readonly id = "assrt" as const;
  readonly #credentials: () => Credentials;
  constructor(credentials: () => Credentials) {
    this.#credentials = credentials;
  }
  async #get(path: string, params: Record<string, string>, signal: AbortSignal) {
    const query = new URLSearchParams({ token: this.#credentials().token, ...params });
    const response = await request(`${ASSRT_BASE}${path}?${query}`, { headers: { "User-Agent": USER_AGENT } }, signal, this.id);
    if (response.status === 401 || response.status === 403) throw new ServiceError("auth", "assrtBadToken");
    if (response.status === 429) throw new ServiceError("quota", "assrtQuota");
    if (!response.ok) throw new ServiceError("network", "assrtError", [response.status]);
    const body = await json(response, this.id);
    const status = num(body.status) ?? 0;
    if (status === 101 || status === 20001) throw new ServiceError("auth", "assrtBadToken");
    if (status === 30900) throw new ServiceError("quota", "assrtQuota");
    if (status !== 0) throw new ServiceError("network", "assrtError", [str(body.errmsg) || status]);
    return body;
  }
  async search(query: Query, signal: AbortSignal) {
    if ([...query.title].length < 3) return { results: [], more: false }; // the API needs 3+ characters
    const count = 15;
    const body = await this.#get("/sub/search", { q: query.title, cnt: String(count), pos: String((query.page - 1) * count) }, signal);
    const subs = list(record(body.sub).subs);
    const results = subs.map((item): Result => {
      const s = record(item);
      const langs = record(record(s.lang).langlist);
      const language = ASSRT_LANGUAGES.find(([key]) => langs[key] === true)?.[1] ?? "zh-CN";
      const name = str(s.native_name) || str(s.videoname) || "subtitle"; // a file name
      const id = num(s.id) ?? 0;
      return {
        key: `assrt:${id}`, service: this.id, language,
        release: str(s.videoname) || name,
        format: (str(s.subtype).split(/[\s/]/)[0] || "?").toLowerCase(),
        fps: null, downloads: null, hashMatch: false, machine: false, hearingImpaired: false,
        url: id ? `https://assrt.net/xml/sub/${Math.floor(id / 1000)}/${id}.xml` : null,
        ref: id, fileName: str(s.filename) || `${name}.srt`,
      };
    });
    return { results: results.filter((r) => r.ref > 0), more: subs.length >= count };
  }
  async download(result: Result, signal: AbortSignal): Promise<Download> {
    const body = await this.#get("/sub/detail", { id: String(result.ref) }, signal);
    const sub = record(list(record(body.sub).subs)[0]);
    const listed = list(sub.filelist).map(record).filter((f) => str(f.url) && str(f.f));
    const targets = listed.length ? listed.map((f) => ({ url: str(f.url), name: str(f.f) })) : [{ url: str(sub.url), name: str(sub.filename) || result.fileName }];
    const files: Download["files"] = [];
    for (const target of targets.slice(0, 16)) {
      if (!target.url) continue;
      const response = await request(target.url, { headers: { "User-Agent": USER_AGENT } }, signal, this.id);
      if (!response.ok) throw new ServiceError("network", "downloadFailed", [response.status]);
      files.push({ name: target.name, bytes: Buffer.from(await response.arrayBuffer()) });
    }
    if (!files.length) throw new ServiceError("network", "assrtNoLink");
    return { files, remaining: null };
  }
  async test(signal: AbortSignal) {
    if (!this.#credentials().token) return line("tokenFirst");
    const body = await this.#get("/user/quota", {}, signal);
    const quota = num(record(body.user).quota);
    return quota === null ? line("tokenValid") : line("tokenRemaining", [quota]);
  }
}
