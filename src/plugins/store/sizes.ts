import { lstat, readdir, stat } from "node:fs/promises";
import { join } from "node:path";

/** One data directory is measured for at most this long (then "—"); tests shorten it. */
export const sizeTiming = { limit: 2000 };

/**
 * Bytes under `directory` (files only, links not followed). Only names and
 * sizes are read, never contents. `null` when the directory is missing or
 * the walk takes longer than `sizeTiming.limit`.
 */
export async function directorySize(directory: string, limit = sizeTiming.limit): Promise<number | null> {
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

/** `12 KB`, `3.4 MB`; `—` when unknown. Units are symbols, not translated. */
export function bytes(size: number | null): string {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
