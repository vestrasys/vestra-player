import { createHash, randomUUID } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { AssetCheck } from "./index-format.ts";
import { get, type Fetch } from "./net.ts";

const MAX_ICON = 512 * 1024;
const MAX_TOTAL = 16 * 1024 * 1024;

/** PNG or JPEG by content, as the player checks it. */
export function imageType(bytes: Uint8Array): "png" | "jpg" | null {
  if (bytes.length > 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return "png";
  if (bytes.length > 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "jpg";
  return null;
}

/**
 * Detail-page icons (design §2.1): a repository's `iconUrl` is downloaded
 * once into `<data>/icons/`, an installed plugin's own icon is copied there
 * (the player shows images only from this plugin's package or data
 * folder). `path()` answers synchronously with what is ready and starts the
 * work otherwise; `ready` is called when a new icon is on disk.
 */
export class Icons {
  readonly #dir: string;
  readonly #fetch: Fetch;
  readonly #ready: () => void;
  readonly #known = new Map<string, string | null | "pending">();

  constructor(dataDirectory: string, fetch: Fetch, ready: () => void) {
    this.#dir = join(dataDirectory, "icons");
    this.#fetch = fetch;
    this.#ready = ready;
  }

  /**
   * The cached file for a remote icon, or `null` (none yet, or it failed).
   * With `check` (the index's iconSha256 / iconSize) the download must match
   * exactly, the cache is keyed by that hash and verified again when used.
   */
  remote(url: string, allowLocalHttp: boolean, check: AssetCheck = null): string | null {
    const key = check ? `sha ${check.sha256}` : `url ${url}`;
    const known = this.#known.get(key);
    if (known !== undefined) return known === "pending" ? null : known;
    const base = join(this.#dir, check ? `sha-${check.sha256}` : digest(url));
    for (const ext of ["png", "jpg"]) {
      try {
        const path = `${base}.${ext}`;
        if (check) {
          const bytes = readFileSync(path);
          if (bytes.length !== check.size || sha256(bytes) !== check.sha256) {
            rmSync(path, { force: true });
            continue;
          }
        } else statSync(path);
        this.#known.set(key, path);
        return path;
      } catch {
        // not cached
      }
    }
    if (check && check.size > MAX_ICON) {
      this.#known.set(key, null);
      return null;
    }
    this.#known.set(key, "pending");
    void get(this.#fetch, url, {
      maxBytes: check ? Math.min(MAX_ICON, check.size) : MAX_ICON,
      ...(check ? { exactSize: check.size, sha256: check.sha256 } : {}),
      timeoutMs: 10_000,
      allowLocalHttp,
    })
      .then((got) => this.#store(key, base, got.bytes!))
      .catch(() => this.#known.set(key, null));
    return null;
  }

  /** A copy of an installed plugin's icon file, or `null`. */
  local(file: string): string | null {
    const key = `file ${file}`;
    const known = this.#known.get(key);
    if (known !== undefined) return known === "pending" ? null : known;
    try {
      const stat = statSync(file);
      if (stat.size > MAX_ICON) throw new Error("too large");
      const base = join(this.#dir, digest(`${file}\n${stat.size}\n${stat.mtimeMs}`));
      return this.#store(key, base, readFileSync(file), false);
    } catch {
      this.#known.set(key, null);
      return null;
    }
  }

  #store(key: string, base: string, bytes: Buffer, notify = true): string | null {
    const type = imageType(bytes);
    if (!type) {
      this.#known.set(key, null);
      return null;
    }
    const path = `${base}.${type}`;
    try {
      mkdirSync(this.#dir, { recursive: true });
      const temporary = `${path}.${randomUUID()}.tmp`;
      writeFileSync(temporary, bytes);
      renameSync(temporary, path);
      this.#trim();
    } catch {
      this.#known.set(key, null);
      return null;
    }
    this.#known.set(key, path);
    if (notify) this.#ready();
    return path;
  }

  /** Over 16 MB: drop the oldest files. */
  #trim() {
    const files = readdirSync(this.#dir).map((name) => {
      const path = join(this.#dir, name);
      const stat = statSync(path);
      return { path, size: stat.size, time: stat.mtimeMs };
    });
    let total = files.reduce((sum, f) => sum + f.size, 0);
    for (const file of files.sort((a, b) => a.time - b.time)) {
      if (total <= MAX_TOTAL) break;
      rmSync(file.path, { force: true });
      total -= file.size;
      for (const [key, value] of this.#known) if (value === file.path) this.#known.delete(key);
    }
  }
}

const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const digest = (text: string) => createHash("sha256").update(text).digest("hex").slice(0, 32);
