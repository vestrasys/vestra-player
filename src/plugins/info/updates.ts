import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, open, rename, rm, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { extname, join } from "node:path";
import type { Command, Plugin } from "@vestra-player/sdk";

/**
 * J10 Check for updates: the player's releases on GitHub. The repository is
 * named here only (packaging/deps.json names the same one for the release
 * tooling); the release workflow publishes `latest.json` with every release.
 */
export const REPOSITORY = "vestrasys/vestra-player";
export const RELEASES_URL = `https://github.com/${REPOSITORY}/releases`;
export const LATEST_URL = `${RELEASES_URL}/latest/download/latest.json`;
/**
 * A release's installer, from that release's own tag (not `releases/latest`):
 * a release published between the check and the download cannot swap the file
 * under the size and sha256 read from `latest.json` (Tauri's updater and
 * electron-updater pin the version the same way).
 */
export const downloadUrl = (version: string, file: string) => `${RELEASES_URL}/download/v${encodeURIComponent(version)}/${encodeURIComponent(file)}`;
/** The release page of `version` (its notes). */
export const releasePage = (version: string) => `${RELEASES_URL}/tag/v${encodeURIComponent(version)}`;

/** The `latest.json` schema major this player reads. */
export const SCHEMA = 1;
/** An installer larger than this is refused (the real ones are about 100 MB). */
export const MAX_INSTALLER = 1024 * 1024 * 1024;
const MAX_JSON = 256 * 1024;
const MAX_REDIRECTS = 5;
const REDIRECT = new Set([301, 302, 303, 307, 308]);

/** Delays of the automatic check; tests shorten them. */
export const timing = {
  /** After activation (the player has started and settled). */
  firstCheck: 30_000,
  /** At most once in this long, across player runs (`lastCheck` is persisted). */
  interval: 24 * 60 * 60 * 1000,
  /** After a failed check, the next automatic one comes this much later instead. */
  retry: 60 * 60 * 1000,
  /** The deadline of `latest.json`. */
  checkTimeout: 20_000,
  /** The deadline of the installer's response headers; the transfer itself has none (it can be cancelled). */
  headerTimeout: 30_000,
  /** A download that receives nothing for this long fails (`stalled`). */
  stallTimeout: 30_000,
};

export type Fetch = (input: string, init: RequestInit) => Promise<Response>;
/** What the updater reaches outside the player: tests replace these. */
export const deps = {
  fetch: ((input, init) => fetch(input, init)) as Fetch,
  now: () => Date.now(),
  /** The folder installers are saved to: the user's Downloads folder. */
  downloads: () => join(homedir(), "Downloads"),
};

/**
 * Why a check or a download failed: a code about.slint words. `detail` is the
 * HTTP status for `http`, else `""`.
 */
export type FailCode =
  | "offline" // DNS, TLS, connection
  | "timeout"
  | "stalled" // the installer's transfer received nothing for timing.stallTimeout
  | "not-found" // 404: no published release (or the repository is private)
  | "rate-limited" // 403 / 429
  | "http" // another status
  | "invalid" // not JSON, or required fields missing or malformed
  | "schema" // a newer latest.json format
  | "size" // the installer's size differs from latest.json
  | "sha256" // the installer's checksum differs from latest.json
  | "disk" // the Downloads folder cannot be written
  | "open" // the installer could not be started
  | "cancelled";
export class UpdateError extends Error {
  readonly code: FailCode;
  readonly detail: string;
  constructor(code: FailCode, message: string, detail = "") {
    super(message);
    this.code = code;
    this.detail = detail;
  }
}

/** One installer of `latest.json`'s `downloads`. */
export interface Installer {
  file: string;
  size: number;
  sha256: string;
}
/** What `latest.json` says, for this platform. */
export interface Release {
  version: string;
  /** `YYYY-MM-DD`, `""` when not given. */
  published: string;
  /** The release notes page. */
  notesUrl: string;
  /** This platform's installer; `null` when the release has none (the release page is offered instead). */
  installer: Installer | null;
}

type Semver = { core: [number, number, number]; pre: string[] };
function semver(version: string): Semver | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z.-]+)?$/.exec(version.trim());
  if (!match) return null;
  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], pre: match[4] ? match[4].split(".") : [] };
}
/** Semantic version order: negative when `a` is older than `b`; `null` when either is not a version. */
export function compareVersions(a: string, b: string): number | null {
  const x = semver(a);
  const y = semver(b);
  if (!x || !y) return null;
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i]! - y.core[i]!;
  // A pre-release is older than its release.
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length;
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const [p, q] = [x.pre[i]!, y.pre[i]!];
    if (p === q) continue;
    const [pn, qn] = [/^\d+$/.test(p), /^\d+$/.test(q)];
    if (pn && qn) return Number(p) - Number(q);
    if (pn !== qn) return pn ? -1 : 1;
    return p < q ? -1 : 1;
  }
  return x.pre.length - y.pre.length;
}

/** The `downloads` key of this build: `windows-x64`, `macos-arm64`, …; `null` when unknown. */
export function platformKey(os: string, arch: string): string | null {
  const system = os === "windows" || os === "win32" ? "windows" : os === "macos" || os === "darwin" ? "macos" : os === "linux" ? "linux" : null;
  const cpu = arch === "x86_64" || arch === "x64" || arch === "amd64" ? "x64" : arch === "aarch64" || arch === "arm64" ? "arm64" : null;
  return system && cpu ? `${system}-${cpu}` : null;
}

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
/** A bare file name: no folder, nothing hidden, nothing a file system refuses. */
const isFileName = (name: unknown): name is string =>
  typeof name === "string" && name.length > 0 && name.length <= 128 && /^[A-Za-z0-9][A-Za-z0-9._ ()+-]*$/.test(name) && !name.includes("..");

/** The installer type this system opens: `.exe` (the setup program) on Windows, `.dmg` on macOS; `null` elsewhere. */
export function installerExtension(platform: string | null): string | null {
  return platform?.startsWith("windows-") ? ".exe" : platform?.startsWith("macos-") ? ".dmg" : null;
}
/** Whether `file` is an installer this platform may download and open. */
export const isInstallerFor = (file: string, platform: string | null) => {
  const extension = installerExtension(platform);
  return extension !== null && extname(file).toLowerCase() === extension;
};

/**
 * Read `latest.json` for `platform`. Unknown fields are ignored; a schema
 * whose major is not {@link SCHEMA} is `schema`, anything else unusable
 * `invalid`. A malformed entry of another platform does not matter.
 */
export function parseLatest(json: unknown, platform: string | null): Release {
  const invalid = (what: string) => new UpdateError("invalid", `latest.json: ${what}`);
  if (!isObject(json)) throw invalid("not an object");
  const schema = typeof json.schema === "number" ? json.schema : typeof json.schema === "string" ? Number.parseFloat(json.schema) : Number.NaN;
  if (!Number.isFinite(schema)) throw invalid("no schema");
  if (Math.trunc(schema) !== SCHEMA) throw new UpdateError("schema", `latest.json: schema ${String(json.schema)} (this player reads ${SCHEMA})`);
  const version = typeof json.version === "string" ? json.version.trim().replace(/^v/, "") : "";
  if (!semver(version)) throw invalid("no version");
  let published = "";
  if (typeof json.published === "string" && /^\d{4}-\d{2}-\d{2}/.test(json.published) && !Number.isNaN(Date.parse(json.published.slice(0, 10))))
    published = json.published.slice(0, 10);
  const notes = typeof json.notes === "string" ? json.notes.trim() : "";
  const notesUrl = /^https:\/\/[^\s]+$/.test(notes) ? notes : releasePage(version);
  if (json.downloads !== undefined && !isObject(json.downloads)) throw invalid("downloads is not an object");
  const entry = platform && isObject(json.downloads) && Object.hasOwn(json.downloads, platform) ? json.downloads[platform] : undefined;
  let installer: Installer | null = null;
  if (entry !== undefined) {
    if (!isObject(entry)) throw invalid(`downloads.${platform} is not an object`);
    const { file, size, sha256 } = entry;
    if (!isFileName(file) || !isInstallerFor(file, platform)) throw invalid(`downloads.${platform}.file`);
    if (typeof size !== "number" || !Number.isSafeInteger(size) || size <= 0 || size > MAX_INSTALLER) throw invalid(`downloads.${platform}.size`);
    if (typeof sha256 !== "string" || !/^[0-9a-fA-F]{64}$/.test(sha256)) throw invalid(`downloads.${platform}.sha256`);
    installer = { file, size, sha256: sha256.toLowerCase() };
  }
  return { version, published, notesUrl, installer };
}

/** `fetch failed` hides the reason in `cause`. */
function networkError(error: unknown, timedOut: UpdateError | null, cancelled: boolean): UpdateError {
  // The user's Cancel wins over what the cut-off transfer looked like.
  if (cancelled) return new UpdateError("cancelled", "cancelled");
  // A deadline cut the transfer off: that, not what the cut-off body looked like.
  if (timedOut) return timedOut;
  if (error instanceof UpdateError) return error;
  const cause = (error as { cause?: { code?: string; message?: string } }).cause;
  return new UpdateError("offline", cause?.message || (error as Error)?.message || String(error));
}

/**
 * GET over https only (every redirect hop too, at most 5), with a deadline
 * for the headers; `onBody` reads the body of the 2xx answer. 404, 403 / 429
 * and other statuses become `not-found`, `rate-limited` and `http`. With
 * `stallTimeout`, the body must keep arriving: `onBody` calls `alive()` for
 * every chunk, and that long without one aborts with `stalled`.
 */
async function get<T>(
  url: string,
  options: { headerTimeout: number; totalTimeout?: number; stallTimeout?: number; signal?: AbortSignal },
  onBody: (response: Response, alive: () => void) => Promise<T>,
): Promise<T> {
  const controller = new AbortController();
  let timedOut: UpdateError | null = null;
  const expire = (error = new UpdateError("timeout", "timed out")) => {
    timedOut ??= error;
    controller.abort();
  };
  let stall: ReturnType<typeof setTimeout> | undefined;
  const alive = () => {
    if (!options.stallTimeout) return;
    clearTimeout(stall);
    stall = setTimeout(() => expire(new UpdateError("stalled", `no data for ${options.stallTimeout} ms`)), options.stallTimeout);
  };
  const cancel = () => controller.abort();
  if (options.signal?.aborted) throw new UpdateError("cancelled", "cancelled");
  options.signal?.addEventListener("abort", cancel, { once: true });
  const headers = setTimeout(() => expire(), options.headerTimeout);
  const total = options.totalTimeout ? setTimeout(() => expire(), options.totalTimeout) : undefined;
  try {
    let current = url;
    for (let hop = 0; ; hop++) {
      if (!current.startsWith("https://")) throw new UpdateError("offline", `not an https address: ${current}`);
      const response = await deps.fetch(current, { redirect: "manual", signal: controller.signal, headers: { "user-agent": "Vestra-Player-Updater" } });
      if (REDIRECT.has(response.status)) {
        await response.body?.cancel().catch(() => {});
        const location = response.headers.get("location");
        if (!location || hop >= MAX_REDIRECTS) throw new UpdateError("http", `HTTP ${response.status} without a usable redirect`, String(response.status));
        current = new URL(location, current).href;
        continue;
      }
      clearTimeout(headers);
      if (!response.ok) {
        await response.body?.cancel().catch(() => {});
        const status = response.status;
        if (status === 404) throw new UpdateError("not-found", `HTTP 404: ${current}`, "404");
        if (status === 403 || status === 429) throw new UpdateError("rate-limited", `HTTP ${status}`, String(status));
        throw new UpdateError("http", `HTTP ${status}`, String(status));
      }
      alive();
      return await onBody(response, alive);
    }
  } catch (error) {
    throw networkError(error, timedOut, options.signal?.aborted === true);
  } finally {
    clearTimeout(headers);
    clearTimeout(total);
    clearTimeout(stall);
    options.signal?.removeEventListener("abort", cancel);
  }
}

/** Fetch and read `latest.json` for `platform`. */
export async function fetchLatest(platform: string | null): Promise<Release> {
  const text = await get(LATEST_URL, { headerTimeout: timing.checkTimeout, totalTimeout: timing.checkTimeout }, async (response) => {
    const reader = response.body?.getReader();
    const chunks: Uint8Array[] = [];
    let received = 0;
    while (reader) {
      const { done, value } = await reader.read();
      if (done) break;
      received += value.byteLength;
      if (received > MAX_JSON) {
        await reader.cancel().catch(() => {});
        throw new UpdateError("invalid", "latest.json is too large");
      }
      chunks.push(value);
    }
    return Buffer.concat(chunks).toString("utf8");
  });
  let json: unknown;
  try {
    json = JSON.parse(text.replace(/^﻿/, ""));
  } catch {
    throw new UpdateError("invalid", "latest.json is not JSON");
  }
  return parseLatest(json, platform);
}

/** The sha256 of a file (lowercase hex) and its size; `null` when it cannot be read. */
async function digest(path: string): Promise<{ size: number; sha256: string } | null> {
  try {
    const hash = createHash("sha256");
    let size = 0;
    for await (const chunk of createReadStream(path)) {
      size += (chunk as Buffer).length;
      hash.update(chunk as Buffer);
    }
    return { size, sha256: hash.digest("hex") };
  } catch {
    return null;
  }
}
/** Whether `path` is exactly `installer` (size, then checksum). */
export async function matches(path: string, installer: Installer) {
  const info = await stat(path).catch(() => null);
  if (!info?.isFile() || info.size !== installer.size) return false;
  return (await digest(path))?.sha256 === installer.sha256;
}

/** `name.ext`, then `name (1).ext`, `name (2).ext`… (how browsers name a second download of the same file). */
export function numberedName(file: string, n: number) {
  if (n === 0) return file;
  const extension = extname(file);
  return `${file.slice(0, file.length - extension.length)} (${n})${extension}`;
}
/**
 * Where `installer` goes in `folder`: the first of `file`, `file (1)`, …
 * that is free, or one that already is this installer (`ready`). A file of
 * the same name that is something else is never overwritten.
 */
async function destination(folder: string, installer: Installer): Promise<{ path: string; ready: boolean }> {
  for (let n = 0; n < 100; n++) {
    const path = join(folder, numberedName(installer.file, n));
    const info = await stat(path).catch((error: NodeJS.ErrnoException) => (error.code === "ENOENT" ? null : error));
    if (info === null) return { path, ready: false };
    if (info instanceof Error) continue;
    if (await matches(path, installer)) return { path, ready: true };
  }
  throw new UpdateError("disk", `no free name for ${installer.file}`);
}

/**
 * Download `installer` of release `version` into `folder`: to `<name>.part`,
 * its size counted while it streams (cut off past `size`, failed after
 * `timing.stallTimeout` without data) and hashed, then renamed to `<name>`
 * once size and sha256 match; a failure deletes the part file. `<name>` is
 * the release's file name, numbered when another file has it; a file there
 * that already is this installer is kept and not downloaded again.
 */
export async function downloadInstaller(
  installer: Installer,
  version: string,
  folder: string,
  onProgress: (received: number) => void,
  signal: AbortSignal,
): Promise<string> {
  try {
    await mkdir(folder, { recursive: true });
  } catch (error) {
    throw new UpdateError("disk", (error as Error).message);
  }
  let { path: target, ready } = await destination(folder, installer);
  if (ready) {
    onProgress(installer.size);
    return target;
  }
  const part = `${target}.part`;
  let created = false;
  try {
    await get(downloadUrl(version, installer.file), { headerTimeout: timing.headerTimeout, stallTimeout: timing.stallTimeout, signal }, async (response, alive) => {
      const declared = (response.headers.get("content-encoding") ?? "identity").trim().toLowerCase() === "identity" ? response.headers.get("content-length") : null;
      if (declared !== null && Number(declared) !== installer.size) {
        await response.body?.cancel().catch(() => {});
        throw new UpdateError("size", `size ${declared}, expected ${installer.size}`);
      }
      let handle: Awaited<ReturnType<typeof open>>;
      try {
        handle = await open(part, "w");
        created = true;
      } catch (error) {
        await response.body?.cancel().catch(() => {});
        throw new UpdateError("disk", (error as Error).message);
      }
      const hash = createHash("sha256");
      let received = 0;
      try {
        const reader = response.body?.getReader();
        while (reader) {
          const { done, value } = await reader.read();
          if (done) break;
          alive();
          received += value.byteLength;
          if (received > installer.size) {
            await reader.cancel().catch(() => {});
            throw new UpdateError("size", `more than ${installer.size} bytes`);
          }
          hash.update(value);
          try {
            await handle.write(value);
          } catch (error) {
            await reader.cancel().catch(() => {});
            throw new UpdateError("disk", (error as Error).message);
          }
          onProgress(received);
        }
      } finally {
        await handle.close().catch(() => {});
      }
      if (received !== installer.size) throw new UpdateError("size", `${received} bytes, expected ${installer.size}`);
      if (hash.digest("hex") !== installer.sha256) throw new UpdateError("sha256", "checksum mismatch");
    });
    try {
      // A file of that name that appeared while downloading is not replaced either.
      if (await stat(target).then(() => true, () => false)) ({ path: target, ready } = await destination(folder, installer));
      if (ready) await rm(part, { force: true });
      else await rename(part, target);
    } catch (error) {
      throw new UpdateError("disk", (error as Error).message);
    }
    return target;
  } catch (error) {
    if (created) await rm(part, { force: true }).catch(() => {});
    throw error;
  }
}

/** Persisted in `<data>/updates.json`. */
export interface UpdateSettings {
  /** The automatic check (on by default). */
  auto: boolean;
  /** When the last check started (ms since the epoch), automatic or not; 0 = never. */
  lastCheck: number;
  /** Whether that check failed: the next automatic one comes `timing.retry` after it instead of a day later (absent: it did not). */
  lastFailed?: boolean;
  /** The newest version an OSD notice was shown for ("" = none): one notice per version. */
  notified: string;
  /** The last release read, so the About window can show it without going online. */
  latest: Release | null;
}
export const UPDATE_DEFAULTS: UpdateSettings = { auto: true, lastCheck: 0, lastFailed: false, notified: "", latest: null };
const isRelease = (v: unknown): v is Release =>
  isObject(v) &&
  typeof v.version === "string" &&
  typeof v.published === "string" &&
  typeof v.notesUrl === "string" &&
  (v.installer === null ||
    (isObject(v.installer) && isFileName(v.installer.file) && typeof v.installer.size === "number" && typeof v.installer.sha256 === "string"));
export const isUpdateSettings = (v: unknown): v is UpdateSettings =>
  isObject(v) &&
  typeof v.auto === "boolean" &&
  typeof v.lastCheck === "number" &&
  (v.lastFailed === undefined || typeof v.lastFailed === "boolean") &&
  typeof v.notified === "string" &&
  (v.latest === null || isRelease(v.latest));

/**
 * The update state the About window shows (`about.slint` words the codes):
 * `""` not checked yet, `checking`, `current`, `available`, `unknown` (the
 * latest release is known, but this player's version cannot be compared with
 * it), `downloading`, `downloaded`, `failed` (the check) and `download-failed`.
 */
export type UpdateState = "" | "checking" | "current" | "available" | "unknown" | "downloading" | "downloaded" | "failed" | "download-failed";
/** The About window's update properties. */
export interface UpdateProps {
  updateState: UpdateState;
  latestVersion: string;
  latestDate: string;
  /** The action button: `check`, `download`, `release-page` (no installer for this platform), `cancel`, `run`, `""` none. */
  updateButton: string;
  /** Why it failed (`FailCode`), and the HTTP status for `http`. */
  updateError: string;
  updateDetail: string;
  /** Downloaded bytes and the installer's size. */
  received: number;
  total: number;
  /** The installer's path once downloaded. */
  installerPath: string;
  hasNotes: boolean;
}

/**
 * The update check of the player itself (J10). One instance per activation:
 * the automatic check runs `timing.firstCheck` after activation when the
 * last one is a day old (an hour, after a failed one), then again while the
 * plugin runs. Its timer does not keep the plugin running (lazy activation
 * stops an idle plugin; the next activation schedules again from the
 * persisted `lastCheck`). A download marks the "Check for Updates…" command
 * busy, which keeps the plugin running until it ends.
 */
export class Updater {
  readonly plugin: Plugin;
  readonly #settings: { value: UpdateSettings; save(next: UpdateSettings): void };
  #state: UpdateState = "";
  #error: UpdateError | null = null;
  #checking: Promise<void> | null = null;
  /** The user asked for the check in flight (it may have started as the automatic one): its result and any failure are shown. */
  #manual = false;
  #download: AbortController | null = null;
  #received = 0;
  #installer = "";
  /** This system's `downloads` key; `undefined` until asked. */
  #platform: string | null | undefined;
  /** The unreadable player version already logged. */
  #badVersion = "";
  #timer: ReturnType<typeof setTimeout> | undefined;
  #disposed = false;
  #lastDraw = 0;
  command: Command | null = null;

  constructor(plugin: Plugin) {
    this.plugin = plugin;
    let settings: { value: UpdateSettings; save(next: UpdateSettings): void } = { value: UPDATE_DEFAULTS, save: (next) => void (settings.value = next) };
    try {
      settings = plugin.storage.json<UpdateSettings>("updates", { defaults: UPDATE_DEFAULTS, validate: isUpdateSettings });
    } catch {
      // No data directory: nothing is remembered between runs.
    }
    this.#settings = settings;
    const latest = settings.value.latest;
    if (latest) {
      const state = this.#stateFor(latest);
      this.#state = state === "available" || settings.value.lastCheck ? state : "";
    }
    plugin.onDispose(() => {
      this.#disposed = true;
      clearTimeout(this.#timer);
      this.#download?.abort();
    });
  }

  get settings() {
    return this.#settings;
  }
  get state() {
    return this.#state;
  }

  /** `release` against this player; `null` when the player's version cannot be read (logged once). */
  #compare(release: Release) {
    const running = this.plugin.app.version;
    const order = compareVersions(release.version, running);
    if (order === null && this.#badVersion !== running) {
      this.#badVersion = running;
      console.warn(`[info] update check: cannot read this player's version "${running}"; not claiming it is up to date`);
    }
    return order;
  }
  #newer(release: Release) {
    return (this.#compare(release) ?? 0) > 0;
  }
  #stateFor(release: Release): "available" | "current" | "unknown" {
    const order = this.#compare(release);
    return order === null ? "unknown" : order > 0 ? "available" : "current";
  }
  #save(patch: Partial<UpdateSettings>) {
    this.#settings.save({ ...this.#settings.value, ...patch });
  }
  /** This system's `downloads` key, from the player (else from Node). */
  async #platformKey() {
    if (this.#platform !== undefined) return this.#platform;
    try {
      const system = await this.plugin.app.system();
      this.#platform = platformKey(system.os, system.arch);
    } catch {
      this.#platform = platformKey(process.platform, process.arch);
    }
    return this.#platform;
  }
  /** When the next automatic check is due: a day after the last check, an hour after a failed one. */
  #due() {
    const value = this.#settings.value;
    return value.lastCheck + (value.lastFailed ? timing.retry : timing.interval);
  }

  /** Turn the automatic check on or off (the settings page). */
  setAuto(on: boolean) {
    if (this.#settings.value.auto === on) return;
    this.#save({ auto: on });
    this.schedule();
  }

  /** (Re)arm the automatic check: `firstCheck` from now at the earliest, when due at the latest. */
  schedule() {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    if (this.#disposed || !this.#settings.value.auto) return;
    const due = this.#due() - deps.now();
    const delay = Math.min(2 ** 31 - 1, Math.max(timing.firstCheck, due));
    this.#timer = setTimeout(() => void this.autoCheck(), delay);
    // The plugin's connection keeps the process alive; this timer alone must not (a test that never stops the plugin).
    this.#timer.unref?.();
  }

  /** The automatic check: quiet on failure (logged); one OSD notice per new version. */
  async autoCheck() {
    this.#timer = undefined;
    if (this.#disposed || !this.#settings.value.auto) return;
    if (deps.now() < this.#due()) return this.schedule();
    await this.check(false);
    this.schedule();
  }

  /**
   * Check now. `manual`: the user asked; the About window shows "checking"
   * and the result or the failure, also when this joins an automatic check
   * already running. Otherwise a failure is only logged and the window keeps
   * what it showed. Nothing is checked while a download runs, and a check
   * that ends while one runs (it started before) leaves the download, its
   * state, its release and the installer alone.
   */
  check(manual: boolean): Promise<void> {
    if (this.#download) return Promise.resolve();
    if (manual) {
      this.#manual = true;
      this.#set("checking");
    }
    if (this.#checking) return this.#checking;
    this.#save({ lastCheck: deps.now() });
    this.#checking = (async () => {
      const platform = await this.#platformKey();
      let release: Release;
      try {
        release = await fetchLatest(platform);
      } catch (error) {
        const failure = error instanceof UpdateError ? error : new UpdateError("invalid", String(error));
        console.warn(`[info] update check (${this.#manual ? "manual" : "automatic"}): ${failure.code}: ${failure.message}`);
        this.#save({ lastFailed: true });
        if (this.#disposed || this.#download) return;
        if (this.#manual) {
          this.#error = failure;
          this.#set("failed");
        }
        return;
      }
      this.#save({ lastFailed: false });
      if (this.#disposed) return;
      if (this.#download) {
        console.warn(`[info] update check: read ${release.version} while a download runs; left for the next check`);
        return;
      }
      const previous = this.#settings.value.latest;
      this.#save({ latest: release });
      this.#error = null;
      const state = this.#stateFor(release);
      if (state !== "available") {
        this.#installer = "";
        return this.#set(state);
      }
      // A manual check shows it in the window: no notice for this version later either.
      if (!this.#manual && this.#settings.value.notified !== release.version) this.#notify(release);
      this.#save({ notified: release.version });
      // The installer downloaded for the same release stays ready to run.
      const same = previous?.version === release.version && previous.installer?.sha256 === release.installer?.sha256;
      if (this.#installer && same) return this.#set("downloaded");
      this.#installer = "";
      this.#set("available");
    })().finally(() => {
      this.#checking = null;
      this.#manual = false;
    });
    return this.#checking;
  }

  #notify(release: Release) {
    void this.plugin.osd
      // TRANSLATORS: an on-screen notice, once per new version; {} is the version, e.g. 0.6.3.
      .show("Vestra Player {} is available. Open About Vestra Player… to download it.", [release.version])
      .catch(() => {});
  }

  #set(state: UpdateState) {
    this.#state = state;
    this.plugin.update();
  }

  /** The About window's action button. */
  async action() {
    const latest = this.#settings.value.latest;
    switch (this.props().updateButton) {
      case "check":
        return this.check(true);
      case "release-page":
        return this.releaseNotes();
      case "cancel":
        this.#download?.abort();
        return;
      case "run":
        return this.run();
      case "download":
        if (latest) return this.download(latest);
    }
  }

  /** Open the release notes in the browser. */
  async releaseNotes() {
    const latest = this.#settings.value.latest;
    await this.plugin.system.openUrl(latest?.notesUrl ?? `${RELEASES_URL}/latest`).catch((error) => console.warn(`[info] release notes: ${(error as Error).message}`));
  }

  /** Download `release`'s installer for this system, from that release's own address. */
  async download(release: Release) {
    const installer = release.installer;
    if (this.#download || !installer) return;
    const controller = (this.#download = new AbortController());
    this.#received = 0;
    this.#error = null;
    this.#set("downloading");
    // A busy command keeps the plugin running while the transfer runs (lazy activation).
    // TRANSLATORS: why "Check for Updates…" is greyed out: an installer is being downloaded.
    this.command?.set({ busy: true, reason: "Downloading the update…" });
    try {
      // Only what this system opens as an installer: .exe on Windows, .dmg on macOS.
      const platform = await this.#platformKey();
      if (!isInstallerFor(installer.file, platform)) throw new UpdateError("invalid", `${installer.file} is not an installer for ${platform ?? "this system"}`);
      const path = await downloadInstaller(
        installer,
        release.version,
        deps.downloads(),
        (received) => {
          this.#received = received;
          // At most ten window updates a second.
          const now = Date.now();
          if (now - this.#lastDraw >= 100 || received === installer.size) {
            this.#lastDraw = now;
            this.plugin.update();
          }
        },
        controller.signal,
      );
      this.#installer = path;
      this.#set("downloaded");
    } catch (error) {
      const failure = error instanceof UpdateError ? error : new UpdateError("disk", String(error));
      console.warn(`[info] update download: ${failure.code}: ${failure.message}`);
      if (this.#disposed) return;
      this.#received = 0;
      if (failure.code === "cancelled") {
        this.#error = null;
        this.#set("available");
      } else {
        this.#error = failure;
        this.#set("download-failed");
      }
    } finally {
      this.#download = null;
      if (!this.#disposed) this.command?.set({ busy: false, reason: null });
    }
  }

  /** Start the downloaded installer (Windows: the setup program; macOS: the disk image opens). Checked again first. */
  async run() {
    const installer = this.#settings.value.latest?.installer;
    if (!installer || !this.#installer) return;
    const fail = (error: UpdateError) => {
      this.#error = error;
      this.#installer = "";
      this.#set("download-failed");
    };
    if (!isInstallerFor(this.#installer, await this.#platformKey())) return fail(new UpdateError("invalid", `${this.#installer} is not an installer for this system`));
    if (!(await matches(this.#installer, installer))) return fail(new UpdateError("sha256", "the downloaded file changed"));
    try {
      await this.plugin.system.openPath(this.#installer);
    } catch (error) {
      console.warn(`[info] run installer: ${(error as Error).message}`);
      this.#error = new UpdateError("open", (error as Error).message);
      this.#set("download-failed");
    }
  }

  props(): UpdateProps {
    const latest = this.#settings.value.latest;
    const state = this.#state;
    const action =
      state === "checking"
        ? ""
        : state === "downloading"
          ? "cancel"
          : state === "downloaded"
            ? "run"
            : (state === "available" || state === "download-failed") && latest && this.#newer(latest)
              ? latest.installer
                ? "download"
                : "release-page"
              : "check";
    return {
      updateState: state,
      latestVersion: latest?.version ?? "",
      latestDate: latest?.published ?? "",
      updateButton: action,
      updateError: this.#error?.code ?? "",
      updateDetail: this.#error?.detail ?? "",
      received: this.#received,
      total: latest?.installer?.size ?? 0,
      installerPath: this.#installer,
      hasNotes: !!latest,
    };
  }
}
