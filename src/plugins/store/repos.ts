import { createHash, randomUUID, type KeyObject } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { IndexError, parseIndex, remember, replayed, type RepoIndex, type Seen } from "./index-format.ts";
import { clock, hostOf } from "./labels.ts";
import { get, isHttps, isLocalHttp, NetError, type Fetch } from "./net.ts";
import { verifyIndex } from "./signing.ts";

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
};

/**
 * A user-entered repository address, normalized (design §3.1): https only
 * (plain http only for this machine), no fragment, and a root address gets
 * `index.json` appended. The result is also the `PackageSource.url` of what
 * gets installed from it, so "only update from the same source" compares
 * these strings.
 */
export function normalizeRepoUrl(input: string): { url: string; local: boolean } | { error: string } {
  const text = input.trim();
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return { error: "请输入 https:// 开头的仓库地址" };
  }
  const local = isLocalHttp(url.href);
  if (!isHttps(url.href) && !local) return { error: "请输入 https:// 开头的仓库地址" };
  if (url.username || url.password) return { error: "仓库地址里不能带用户名或密码" };
  url.hash = "";
  if (!url.pathname.endsWith(".json")) url.pathname = `${url.pathname.replace(/\/+$/, "")}/index.json`;
  if (url.href.length > 2048) return { error: "仓库地址太长" };
  return { url: url.href, local };
}

export class Repo {
  readonly url: string;
  readonly official: boolean;
  readonly local: boolean;
  index: RepoIndex | null = null;
  /** When the copy in use was last confirmed by the server (200 or 304). */
  fetchedAt: number | null = null;
  status: { text: string; warn: boolean } = { text: "", warn: false };
  refreshing = false;
  meta: Meta | null = null;

  constructor(url: string, official: boolean) {
    this.url = url;
    this.official = official;
    this.local = isLocalHttp(url);
  }

  /** 「官方仓库」, or the index's own name with the host, so a third party cannot pose as us. */
  get label(): string {
    if (this.official) return "官方仓库";
    return `${this.index?.name || "未命名仓库"}（${hostOf(this.url)}）`;
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

  async refreshAll(force: boolean): Promise<void> {
    const queue = this.list.filter((repo) => force || this.stale(repo));
    const workers = Array.from({ length: Math.min(4, queue.length) }, async () => {
      for (let repo = queue.shift(); repo; repo = queue.shift()) await this.refresh(repo);
    });
    await Promise.all(workers);
  }

  stale(repo: Repo): boolean {
    return repo.fetchedAt === null || this.#deps.now() - repo.fetchedAt >= STALE_MS;
  }

  /** Fetch a repository's index; on any failure keep the copy we have. */
  async refresh(repo: Repo): Promise<void> {
    if (repo.refreshing) return;
    repo.refreshing = true;
    repo.status = { text: `正在更新 ${repo.label}…`, warn: false };
    try {
      const result = await this.#fetch(repo, repo.index !== null);
      const now = this.#deps.now();
      if (result === "unchanged") {
        repo.fetchedAt = now;
        if (repo.meta) this.#writeMeta(repo, { ...repo.meta, fetchedAt: now });
        repo.status = { text: `${repo.label} ${clock(now, now)} 已是最新`, warn: false };
      } else {
        repo.index = result.index;
        repo.fetchedAt = now;
        this.#save(repo, result.bytes, result.signature, {
          url: repo.url,
          etag: result.etag,
          lastModified: result.lastModified,
          fetchedAt: now,
          ...(repo.official ? { seen: remember(repo.meta?.seen, result.index) } : {}),
        });
        repo.status = { text: `${repo.label} ${clock(now, now)} 已更新`, warn: false };
      }
    } catch (error) {
      repo.status = { text: this.#failure(repo, error), warn: true };
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

  async #fetch(repo: Repo, conditional: boolean) {
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
        headers: ask,
        allowLocalHttp: repo.local,
      });
      if (got.status === 304) {
        if (repo.index) return "unchanged" as const;
        if (attempt === 0) continue;
        throw new NetError("http", "HTTP 304", 304);
      }
      const bytes = got.bytes!;
      let signature: string | null = null;
      if (repo.official) {
        try {
          const sig = await get(this.#deps.fetch, `${repo.url}.sig`, {
            maxBytes: SIG_LIMIT,
            timeoutMs: 15_000,
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
        const lost = replayed(repo.meta?.seen, index);
        if (lost) {
          this.#deps.log(`${repo.url}: replayed index, ${lost} is gone`);
          throw new StoreRefusal("replay");
        }
      }
      return { index, bytes, signature, etag: got.etag, lastModified: got.lastModified };
    }
  }

  #failure(repo: Repo, error: unknown): string {
    const cached = repo.index && repo.fetchedAt !== null ? `，继续显示 ${clock(repo.fetchedAt, this.#deps.now())} 的数据` : "";
    if (error instanceof StoreRefusal) {
      const text = {
        signatureMissing: "官方仓库缺少签名文件，已忽略这次更新",
        signature: "官方仓库的签名校验失败，可能被篡改，已忽略这次更新",
        replay: "官方仓库返回了旧的索引，已忽略这次更新",
      }[error.kind];
      return text;
    }
    if (error instanceof IndexError)
      return error.kind === "schema"
        ? `${repo.label} 使用更新的格式，请更新 Vestra Player`
        : `${repo.label} 的索引格式不正确${cached}`;
    if (error instanceof NetError) {
      if (error.code === "http") return `${repo.label} 返回错误 ${error.status}${cached}`;
      if (error.code === "tooLarge") return `${repo.label} 的索引超过 4 MB，已拒绝${cached}`;
      if (error.code === "protocol" || error.code === "redirects") return `${repo.label}：${error.message}${cached}`;
      if (!repo.index) return `无法连接 ${repo.label}（${error.message}），暂时看不到它的插件`;
      return `无法连接 ${repo.label}（${error.message}），显示 ${clock(repo.fetchedAt ?? 0, this.#deps.now())} 的缓存`;
    }
    return `${repo.label}：${(error as Error).message}${cached}`;
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
      repo.index = parseIndex(bytes.toString("utf8"), { allowLocalHttp: repo.local }).index;
      repo.meta = meta;
      repo.fetchedAt = Number.isFinite(meta.fetchedAt) ? meta.fetchedAt : null;
      if (repo.fetchedAt !== null)
        repo.status = { text: `${repo.label}：${clock(repo.fetchedAt, this.#deps.now())} 的数据`, warn: false };
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
