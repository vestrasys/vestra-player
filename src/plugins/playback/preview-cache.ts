import { createHash } from "node:crypto";
import { readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * I07 (0.2.0): sprites are kept across sessions in `dataDirectory/thumbs/<key>/`
 * (`sheet-0.jpg`, `sheet-1.jpg`, `index.json`), at most `CACHE_LIMIT_BYTES`
 * in all; the least recently used go first. A file that changed (size or
 * modification time) gets a new key, so its old sprite simply ages out.
 */
export const CACHE_VERSION = 2;
export const CACHE_LIMIT_BYTES = 200 * 1024 * 1024;
export const INDEX_FILE = "index.json";

export type FileFacts = { path: string; size: number; mtimeMs: number };

/** The folder name of a source's sprite: its real path, size, modification time and cell count. */
export function cacheKey(file: FileFacts, count: number): string {
  return createHash("sha1")
    .update(`${CACHE_VERSION}\n${file.path}\n${file.size}\n${file.mtimeMs}\n${count}`)
    .digest("hex")
    .slice(0, 20);
}

/** `index.json`: which cells the sheets hold. */
export type CacheIndex = {
  version: number;
  count: number;
  /** Cells made (in the sheets). */
  made: number[];
  /** Cells ffmpeg could not make; not tried again for this file. */
  failed: number[];
  /** Every cell is made or failed. */
  complete: boolean;
  /** Last opened (ms since the epoch), for eviction. */
  usedAt: number;
};

const isIndex = (value: unknown): value is CacheIndex => {
  const v = value as Partial<CacheIndex> | null;
  return (
    typeof v === "object" &&
    v !== null &&
    v.version === CACHE_VERSION &&
    Number.isInteger(v.count) &&
    Array.isArray(v.made) &&
    v.made.every((i) => Number.isInteger(i) && i >= 0 && i < v.count!) &&
    Array.isArray(v.failed) &&
    v.failed.every((i) => Number.isInteger(i) && i >= 0 && i < v.count!) &&
    typeof v.complete === "boolean" &&
    typeof v.usedAt === "number"
  );
};

/** The index of a sprite folder, `null` when missing, broken or of another version. */
export async function readIndex(dir: string): Promise<CacheIndex | null> {
  try {
    const value: unknown = JSON.parse(await readFile(join(dir, INDEX_FILE), "utf8"));
    return isIndex(value) ? value : null;
  } catch {
    return null;
  }
}

export const writeIndex = (dir: string, index: CacheIndex) =>
  writeFile(join(dir, INDEX_FILE), JSON.stringify(index));

async function folderBytes(dir: string): Promise<number> {
  let total = 0;
  for (const name of await readdir(dir).catch(() => [] as string[]))
    total += await stat(join(dir, name)).then(
      (s) => (s.isFile() ? s.size : 0),
      () => 0,
    );
  return total;
}

/**
 * Keep `root` under `limit` bytes: folders without a valid index (an older
 * version's pictures, a broken write) are removed, then the least recently
 * used sprites until the rest fits. `keep` (the sources in use) is never removed.
 * Returns the removed folder names.
 */
export async function evict(root: string, keep: ReadonlySet<string>, limit = CACHE_LIMIT_BYTES): Promise<string[]> {
  const removed: string[] = [];
  const entries: { name: string; usedAt: number; bytes: number }[] = [];
  let total = 0;
  for (const name of await readdir(root).catch(() => [] as string[])) {
    const dir = join(root, name);
    const index = await readIndex(dir);
    if (!index) {
      if (keep.has(name)) continue; // being made: its index comes with the first sheet
      await rm(dir, { recursive: true, force: true }).catch(() => {});
      removed.push(name);
      continue;
    }
    const bytes = await folderBytes(dir);
    total += bytes;
    entries.push({ name, usedAt: index.usedAt, bytes });
  }
  entries.sort((a, b) => a.usedAt - b.usedAt);
  for (const entry of entries) {
    if (total <= limit) break;
    if (keep.has(entry.name)) continue;
    await rm(join(root, entry.name), { recursive: true, force: true }).catch(() => {});
    total -= entry.bytes;
    removed.push(entry.name);
  }
  return removed;
}
