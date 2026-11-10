import { createHash } from "node:crypto";
import { readdir, readFile, rename, rm, stat, unlink, writeFile } from "node:fs/promises";
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

/**
 * The folder name of a source's sprite: its real path, size, modification
 * time, cell count and (0.2.2) how it was made. Keyframe sprites keep their
 * 0.2.0 names, so an upgrade throws none away; a file that now gets exact
 * cells gets a new folder, and its keyframe one ages out.
 */
export function cacheKey(file: FileFacts, count: number, mode: "keyframe" | "exact" = "keyframe"): string {
  return createHash("sha1")
    .update(`${CACHE_VERSION}\n${file.path}\n${file.size}\n${file.mtimeMs}\n${count}${mode === "exact" ? "\nexact" : ""}`)
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
  /**
   * 0.2.2, exact folders only: the exact pass failed or was too slow here, so
   * the file gets the keyframe sprite and the pass is not tried again.
   */
  keyframesOnly?: boolean;
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
    typeof v.usedAt === "number" &&
    (v.keyframesOnly === undefined || typeof v.keyframesOnly === "boolean")
  );
};

/**
 * 0.2.12: `index.json` is written as `index.json.<pid>.<n>.tmp` in the same
 * folder, then renamed over it, so a reader sees the old index or the new one,
 * never half of one. Readers only read `index.json`; eviction leaves a folder
 * alone while such a write is fresh (`WRITE_GRACE_MS`).
 */
export const INDEX_TEMP = /^index\.json\.\d+\.\d+\.tmp$/;
/** A temporary index younger than this is a write in progress; an older one is left over from a crash. */
export const WRITE_GRACE_MS = 60_000;
/**
 * Renames refused for a moment on Windows are tried this often. A rename over
 * a file that another handle has open (a reader of the index or a sheet, a
 * virus scanner, the indexer) fails with EPERM, EACCES or EBUSY until that
 * handle closes. 0.2.18: the waits grow by `RENAME_STEP_MS` up to
 * `RENAME_MAX_WAIT_MS` (the steps of graceful-fs's win32 rename), so all
 * tries span about 1 s. The ~1 s total and retrying over an existing, open
 * target are this plugin's choice (graceful-fs retries for up to 60 s and only
 * while the target does not exist; Windows refuses to replace a file another
 * handle has open). Before, 5 tries over 0.2 s still lost to a busy reader now
 * and then.
 */
export const RENAME_TRIES = 15;
export const RENAME_STEP_MS = 10;
export const RENAME_MAX_WAIT_MS = 100;
const TRANSIENT = new Set(["EPERM", "EACCES", "EBUSY"]);
let temps = 0;
/** The file calls of `writeIndex` and `replaceFile`; tests replace them to play a failing disk or a refused rename. */
export const indexFs = { writeFile, rename };

/** The wait after the `attempt`th refused rename (1-based). */
export const renameWait = (attempt: number): number => Math.min(RENAME_STEP_MS * attempt, RENAME_MAX_WAIT_MS);

/**
 * Rename `from` over `to`, trying again while Windows refuses it for a moment
 * (see `RENAME_TRIES`). Node's rename replaces an existing file on Windows too
 * (libuv: MoveFileExW with MOVEFILE_REPLACE_EXISTING). The last error is
 * thrown; `from` is left for the caller to remove.
 */
export async function replaceFile(from: string, to: string): Promise<void> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await indexFs.rename(from, to);
    } catch (error) {
      if (attempt >= RENAME_TRIES || !TRANSIENT.has((error as NodeJS.ErrnoException).code ?? "")) throw error;
      await new Promise((resolve) => setTimeout(resolve, renameWait(attempt)));
    }
  }
}

type Look = { index: CacheIndex } | { invalid: true } | { unreadable: true };

/** What a folder's index says: valid, invalid (missing, broken, another version) or not readable just now. */
async function look(dir: string): Promise<Look> {
  let text: string;
  try {
    text = await readFile(join(dir, INDEX_FILE), "utf8");
  } catch (error) {
    // Missing: no index. Anything else (a sharing violation while it is being
    // replaced, too many open files): ask again later.
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? { invalid: true } : { unreadable: true };
  }
  try {
    const value: unknown = JSON.parse(text);
    return isIndex(value) ? { index: value } : { invalid: true };
  } catch {
    return { invalid: true };
  }
}

/** The index of a sprite folder, `null` when missing, broken or of another version. */
export async function readIndex(dir: string): Promise<CacheIndex | null> {
  const found = await look(dir);
  return "index" in found ? found.index : null;
}

/** Write a sprite folder's index atomically: a temporary file in the folder, renamed over `index.json`. */
export async function writeIndex(dir: string, index: CacheIndex): Promise<void> {
  const temp = join(dir, `${INDEX_FILE}.${process.pid}.${++temps}.tmp`);
  try {
    // Inside the try: a write that fails half way leaves no temporary file behind.
    await indexFs.writeFile(temp, JSON.stringify(index));
    // Refused while a reader (a clean-up looking at this folder, a virus
    // scanner) has index.json open: tried again for about 1 s.
    await replaceFile(temp, join(dir, INDEX_FILE));
  } catch (error) {
    await unlink(temp).catch(() => {});
    throw error;
  }
}

/** The folder has a temporary index written less than `WRITE_GRACE_MS` before `now`: a write is in progress. */
async function writing(dir: string, now: number): Promise<boolean> {
  for (const name of await readdir(dir).catch(() => [] as string[])) {
    if (!INDEX_TEMP.test(name)) continue;
    const fresh = await stat(join(dir, name)).then(
      (s) => now - s.mtimeMs < WRITE_GRACE_MS,
      () => false,
    );
    if (fresh) return true;
  }
  return false;
}

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
 * used sprites until the rest fits. `keep` (the sources in use) is never removed
 * (asked for each folder as it is looked at, so it may change meanwhile),
 * nor (0.2.12) a folder whose index cannot be read just now or is being
 * written (a fresh temporary index): the next clean-up looks again.
 * Returns the removed folder names.
 */
export async function evict(root: string, keep: { has(name: string): boolean }, limit = CACHE_LIMIT_BYTES): Promise<string[]> {
  const removed: string[] = [];
  const entries: { name: string; usedAt: number; bytes: number }[] = [];
  let total = 0;
  for (const name of await readdir(root).catch(() => [] as string[])) {
    const dir = join(root, name);
    const found = await look(dir);
    if (!("index" in found)) {
      if (keep.has(name)) continue; // being made: its index comes with the first sheet
      if ("unreadable" in found || (await writing(dir, Date.now()))) continue;
      await rm(dir, { recursive: true, force: true }).catch(() => {});
      removed.push(name);
      continue;
    }
    const bytes = await folderBytes(dir);
    total += bytes;
    entries.push({ name, usedAt: found.index.usedAt, bytes });
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
