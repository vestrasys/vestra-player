import { createHash, randomUUID, type KeyObject } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { IndexError, parseIndex, remember, replayed, type RepoIndex, type Seen } from "./index-format.ts";
import { clock, hostOf } from "./labels.ts";
import { get, isHttps, isLocalHttp, NetError, type Fetch } from "./net.ts";
import { verifyIndex } from "./signing.ts";
import { msg, raw, type Msg } from "./text.ts";

/** What the store needs from the outside world; tests pass their own. */
export interface Deps {
  fetch: Fetch;
  /** Trusted keys of the official index. */
  keys: KeyObject[];
  now: () => number;
  officialUrl: string;
  userAgent: string;
  log: (line: string) => void;
}

export const OFFICIAL_URL = "https://vestrasys.github.io/vestra-player-plugins/index.json";
export const INDEX_LIMIT = 4 * 1024 * 1024;
const SIG_LIMIT = 1024;
export const MAX_REPOS = 16;
/** Opening the store refreshes a repository whose copy is older than this (Pages' max-age). */
export const STALE_MS = 10 * 60 * 1000;

type Meta = {
  url: string;
  etag: string | null;
  lastModified: string | null;
  fetchedAt: number;
  /** Official repository only: what its signed indexes have shown (never shrinks). */
  seen?: Seen;
  /** Official repository only: the highest `generated` accepted (never shrinks). */
  generated?: number;
};

/**
 * A user-entered repository address, normalized (design §3.1): https only
 * (plain http only for this machine), no fragment, and a root address gets
 * `index.json` appended. The result is also the `PackageSource.url` of what
 * gets installed from it, so "only update from the same source" compares
 * these strings.
 */
export function normalizeRepoUrl(input: string): { url: string; local: boolean } | { error: Msg } {
  const text = input.trim();
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return { error: msg("Enter a repository address starting with https://") };
  }
  const local = isLocalHttp(url.href);
  if (!isHttps(url.href) && !local) return { error: msg("Enter a repository address starting with https://") };
  if (url.username || url.password) return { error: msg("The address cannot contain a user name or password") };
  url.hash = "";
  if (!url.pathname.endsWith(".json")) url.pathname = `${url.pathname.replace(/\/+$/, "")}/index.json`;
  if (url.href.length > 2048) return { error: msg("The repository address is too long") };
  return { url: url.href, local };
}

export class Repo {
  readonly url: string;
  readonly official: boolean;
  readonly local: boolean;
  index: RepoIndex | null = null;
  /** When the copy in use was last confirmed by the server (200 or 304). */
  fetchedAt: number | null = null;
  status: { text: Msg | null; warn: boolean } = { text: null, warn: false };
  refreshing = false;
  meta: Meta | null = null;

  constructor(url: string, official: boolean) {
    this.url = url;
    this.official = official;
    this.local = isLocalHttp(url);
  }

  /** "Official repository", or the index's own name with the host, so a third party cannot pose as us. */
  get label(): Msg {
    if (this.official) return msg("Official repository");
    const name = this.index?.name;
    return name ? msg("{} ({})", name, hostOf(this.url)) : msg("Unnamed repository ({})", hostOf(this.url));
  }

  /** "Unofficial: <label>" (the official repository: "Official repository"). */
  get origin(): Msg {
    if (this.official) return msg("Official repository");
    const name = this.index?.name;
    return name
      ? msg("Unofficial: {} ({})", name, hostOf(this.url))
      : msg("Unofficial: Unnamed repository ({})", hostOf(this.url));
  }
}

/** The official repository and the user's third-party ones, with their caches. */
export class Repos {
  readonly #deps: Deps;
  readonly #root: string;
  list: Repo[];

  constructor(deps: Deps, dataDirectory: string) {
    this.#deps = deps;
    this.#root = join(dataDirectory, "repos");
    const official = new Repo(deps.officialUrl, true);
    this.#load(official);
    this.list = [official];
  }

  get official(): Repo {
    return this.list[0]!;
  }

  find(url: string): Repo | undefined {
    return this.list.find((repo) => repo.url === url);
  }

  /** Follow the settings: keep known repositories, load the cache of new ones. */
  sync(thirdParty: readonly string[]) {
    const next = [this.official];
    for (const url of thirdParty) {
      if (url === this.official.url || next.some((repo) => repo.url === url)) continue;
      let repo = this.find(url);
      if (!repo) {
        repo = new Repo(url, false);
        this.#load(repo);
      }
      next.push(repo);
    }
    this.list = next;
  }

  /** Forget a removed repository's cache. */
  forget(url: string) {
    rmSync(this.#dir(url), { recursive: true, force: true });
  }

  /** `headerTimeoutMs`: a shorter wait for the response headers (opening the window must not spin for long offline). */
  async refreshAll(force: boolean, headerTimeoutMs?: number): Promise<void> {
    const queue = this.list.filter((repo) => force || this.stale(repo));
    const workers = Array.from({ length: Math.min(4, queue.length) }, async () => {
      for (let repo = queue.shift(); repo; repo = queue.shift()) await this.refresh(repo, headerTimeoutMs);
    });
    await Promise.all(workers);
  }

  stale(repo: Repo): boolean {
    return repo.fetchedAt === null || this.#deps.now() - repo.fetchedAt >= STALE_MS;
  }

  /** Fetch a repository's index; on any failure keep the copy we have. */
  async refresh(repo: Repo, headerTimeoutMs?: number): Promise<void> {
    if (repo.refreshing) return;
    repo.refreshing = true;
    repo.status = { text: msg("Updating {}…", repo.label), warn: false };
    try {
      const result = await this.#fetch(repo, repo.index !== null, headerTimeoutMs);
      const now = this.#deps.now();
      if (result === "unchanged") {
        repo.fetchedAt = now;
        if (repo.meta) this.#writeMeta(repo, { ...repo.meta, fetchedAt: now });
        repo.status = { text: msg("{0}: up to date at {1}", repo.label, clock(now, now)), warn: false };
      } else {
        repo.index = result.index;
        repo.fetchedAt = now;
        this.#save(repo, result.bytes, result.signature, {
          url: repo.url,
          etag: result.etag,
          lastModified: result.lastModified,
          fetchedAt: now,
          ...(repo.official
            ? {
                seen: remember(repo.meta?.seen, result.index),
                generated: Math.max(repo.meta?.generated ?? 0, result.index.generated),
              }
            : {}),
        });
        repo.status = { text: msg("{0}: updated at {1}", repo.label, clock(now, now)), warn: false };
      }
    } catch (error) {
      // Being offline is not a fault: said calmly (the other failures are warnings).
      const offline = error instanceof NetError && (error.code === "network" || error.code === "timeout");
      repo.status = { text: this.#failure(repo, error), warn: !offline };
      this.#deps.log(`${repo.url}: ${(error as Error).message}`);
    } finally {
      repo.refreshing = false;
    }
  }

  /** Read a repository being added, without keeping anything (design §4). */
  async probe(url: string): Promise<RepoIndex> {
    const repo = new Repo(url, false);
    const result = await this.#fetch(repo, false);
    if (result === "unchanged") throw new Error("unexpected 304");
    return result.index;
  }

  async #fetch(repo: Repo, conditional: boolean, headerTimeoutMs?: number) {
    const wait = headerTimeoutMs === undefined ? {} : { headerTimeoutMs };
    const headers: Record<string, string> = { "User-Agent": this.#deps.userAgent };
    // The official index and its signature are fetched as a pair: when the
    // index changed between the two requests, one more try usually matches.
    for (let attempt = 0; ; attempt++) {
      const ask = { ...headers };
      if (conditional && attempt === 0 && repo.meta?.url === repo.url) {
        if (repo.meta.etag) ask["If-None-Match"] = repo.meta.etag;
        if (repo.meta.lastModified) ask["If-Modified-Since"] = repo.meta.lastModified;
      }
      const got = await get(this.#deps.fetch, repo.url, {
        maxBytes: INDEX_LIMIT,
        timeoutMs: 30_000,
        ...wait,
        headers: ask,
        allowLocalHttp: repo.local,
      });
      if (got.status === 304) {
        if (repo.index) return "unchanged" as const;
        if (attempt === 0) continue;
        throw new NetError("http", raw("HTTP 304"), 304);
      }
      const bytes = got.bytes!;
      let signature: string | null = null;
      if (repo.official) {
        try {
          const sig = await get(this.#deps.fetch, `${repo.url}.sig`, {
            maxBytes: SIG_LIMIT,
            timeoutMs: 15_000,
            ...wait,
            headers,
          });
          signature = sig.bytes!.toString("utf8");
        } catch (error) {
          if (error instanceof NetError && error.status === 404) throw new StoreRefusal("signatureMissing");
          throw error;
        }
        if (!verifyIndex(bytes, signature, this.#deps.keys)) {
          if (attempt === 0) continue;
          throw new StoreRefusal("signature");
        }
      }
      // Only bytes that verified are parsed (never re-serialized).
      const { index, dropped } = parseIndex(bytes.toString("utf8"), { allowLocalHttp: repo.local });
      if (dropped.length) this.#deps.log(`${repo.url}: ignored ${dropped.slice(0, 20).join(", ")}`);
      if (repo.official) {
        // A lower sequence number than one accepted before (an index without
        // `generated` counts as 0, so it is refused once one was seen).
        const accepted = repo.meta?.generated ?? 0;
        if (index.generated < accepted) {
          this.#deps.log(`${repo.url}: replayed index, generated ${index.generated} < ${accepted}`);
          throw new StoreRefusal("replay");
        }
        const lost = replayed(repo.meta?.seen, index);
        if (lost) {
          this.#deps.log(`${repo.url}: replayed index, ${lost} is gone`);
          throw new StoreRefusal("replay");
        }
      }
      return { index, bytes, signature, etag: got.etag, lastModified: got.lastModified };
    }
  }

  #failure(repo: Repo, error: unknown): Msg {
    const label = repo.label;
    // Since when the copy still shown is, if there is one.
    const cached = repo.index && repo.fetchedAt !== null ? clock(repo.fetchedAt, this.#deps.now()) : null;
    if (error instanceof StoreRefusal) {
      if (error.kind === "signatureMissing")
        return msg("The official repository has no signature file; this update was ignored");
      if (error.kind === "signature")
        return msg(
          "The official repository's signature check failed (it may have been tampered with); this update was ignored",
        );
      return msg("The official repository returned an old index; this update was ignored");
    }
    if (error instanceof IndexError) {
      if (error.kind === "schema") return msg("{} uses a newer format; please update Vestra Player", label);
      return cached === null
        ? msg("{}'s index is malformed", label)
        : msg("{0}'s index is malformed; showing the data from {1}", label, cached);
    }
    if (error instanceof NetError) {
      const status = String(error.status);
      if (error.code === "http")
        return cached === null
          ? msg("{0} returned error {1}", label, status)
          : msg("{0} returned error {1}; showing the data from {2}", label, status, cached);
      if (error.code === "tooLarge")
        return cached === null
          ? msg("{}'s index exceeds 4 MB; refused", label)
          : msg("{0}'s index exceeds 4 MB; refused; showing the data from {1}", label, cached);
      if (error.code === "protocol" || error.code === "redirects")
        return cached === null
          ? msg("{0}: {1}", label, error.reason)
          : msg("{0}: {1}; showing the data from {2}", label, error.reason, cached);
      if (!repo.index) return msg("Can't connect to {0} ({1}); its plugins are not available for now", label, error.reason);
      return msg(
        "Can't connect to {0} ({1}); showing the cache from {2}",
        label,
        error.reason,
        clock(repo.fetchedAt ?? 0, this.#deps.now()),
      );
    }
    const reason = raw((error as Error).message);
    return cached === null
      ? msg("{0}: {1}", label, reason)
      : msg("{0}: {1}; showing the data from {2}", label, reason, cached);
  }

  #dir(url: string) {
    return join(this.#root, createHash("sha256").update(url).digest("hex").slice(0, 16));
  }

  /** The cached copy; the official one is verified again before use. */
  #load(repo: Repo) {
    const dir = this.#dir(repo.url);
    try {
      const meta = JSON.parse(readFileSync(join(dir, "meta.json"), "utf8")) as Meta;
      if (meta.url !== repo.url) return;
      const bytes = readFileSync(join(dir, "index.json"));
      if (repo.official) {
        const signature = readFileSync(join(dir, "index.json.sig"), "utf8");
        if (!verifyIndex(bytes, signature, this.#deps.keys)) {
          this.#deps.log(`${repo.url}: cached index does not verify, ignored`);
          return;
        }
      }
      const index = parseIndex(bytes.toString("utf8"), { allowLocalHttp: repo.local }).index;
      if (repo.official && index.generated < (meta.generated ?? 0)) {
        this.#deps.log(`${repo.url}: cached index is older than one accepted before, ignored`);
        return;
      }
      repo.index = index;
      repo.meta = meta;
      repo.fetchedAt = Number.isFinite(meta.fetchedAt) ? meta.fetchedAt : null;
      if (repo.fetchedAt !== null)
        repo.status = { text: msg("{0}: data from {1}", repo.label, clock(repo.fetchedAt, this.#deps.now())), warn: false };
    } catch {
      // No cache yet, or unreadable: start without one.
    }
  }

  #save(repo: Repo, bytes: Buffer, signature: string | null, meta: Meta) {
    repo.meta = meta;
    try {
      const dir = this.#dir(repo.url);
      mkdirSync(dir, { recursive: true });
      atomic(join(dir, "index.json"), bytes);
      if (signature !== null) atomic(join(dir, "index.json.sig"), signature);
      atomic(join(dir, "meta.json"), JSON.stringify(meta));
    } catch (error) {
      this.#deps.log(`cache ${repo.url}: ${(error as Error).message}`);
    }
  }

  #writeMeta(repo: Repo, meta: Meta) {
    repo.meta = meta;
    try {
      atomic(join(this.#dir(repo.url), "meta.json"), JSON.stringify(meta));
    } catch {
      // Only the time of the last check is lost.
    }
  }
}

class StoreRefusal extends Error {
  readonly kind: "signatureMissing" | "signature" | "replay";
  constructor(kind: "signatureMissing" | "signature" | "replay") {
    super(kind);
    this.kind = kind;
  }
}

/** Temporary file and rename: an interrupted write never leaves half a file. */
function atomic(path: string, data: string | Buffer) {
  const temporary = `${path}.${randomUUID()}.tmp`;
  writeFileSync(temporary, data);
  try {
    renameSync(temporary, path);
  } catch (error) {
    rmSync(temporary, { force: true });
    throw error;
  }
}
