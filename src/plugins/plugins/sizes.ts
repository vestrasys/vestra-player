import { lstat, readdir, stat } from "node:fs/promises";
import { join } from "node:path";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** J07: one data directory is measured for at most this long (then 「—」). */
  sizeLimit: 2000,
  /** List changes within this window are measured once. */
  sizeDebounce: 300,
};

/**
 * Bytes under `directory` (files only, links not followed). Only names and
 * sizes are read, never contents. `null` when the directory is missing or
 * the walk takes longer than `timing.sizeLimit`.
 */
export async function directorySize(directory: string, limit = timing.sizeLimit): Promise<number | null> {
  const deadline = Date.now() + limit;
  try {
    if (!(await stat(directory)).isDirectory()) return null;
  } catch {
    return null;
  }
  let total = 0;
  const pending = [directory];
  while (pending.length) {
    if (Date.now() > deadline) return null;
    const dir = pending.pop()!;
    let entries;
    try {
      entries = await readdir(dir, { withFileTypes: true });
    } catch {
      continue; // removed or unreadable meanwhile
    }
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) total += await lstat(path).then((info) => info.size, () => 0);
    }
  }
  return total;
}
