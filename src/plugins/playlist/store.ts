import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { OpenMode } from "./follow.ts";
import { lengthOf, ROW_LIMIT, type Queue, type Repeat, type SortKey } from "./queue.ts";

/** C17 settings, saved even when the list is not remembered. */
export type Settings = {
  remember: boolean;
  repeat: Repeat;
  shuffle: boolean;
  openMode: OpenMode;
  /** B26 (not implemented yet); kept so a saved value survives. */
  gapless: boolean;
  /** A03: clear the list before "打开文件夹" adds (no menu entry). */
  clearOnOpenFolder: boolean;
};
type SavedList = {
  sort: { key: Exclude<SortKey, "shuffle" | "reverse">; descending: boolean } | null;
  cursor: number | null;
  selected: number[];
  rows: { kind: "file" | "url"; source: string; title: string | null; duration: number | null }[];
};
type Saved = { version: 1; settings: Settings; list?: SavedList };

export const defaults = (): Settings => ({
  remember: true,
  repeat: "none",
  shuffle: false,
  openMode: "only",
  gapless: false,
  clearOnOpenFolder: false,
});

const REPEATS = ["none", "all", "one"];
const MODES = ["only", "similar", "folder"];
const SORT_KEYS = ["title", "filename", "extension", "size", "duration", "date", "folder", "path"];

/** Write `data` atomically (temporary file, then rename). */
export function writeAtomic(path: string, data: string | Uint8Array) {
  const temporary = `${path}.tmp-${process.pid}`;
  writeFileSync(temporary, data);
  renameSync(temporary, path);
}

function readSettings(raw: Partial<Settings> | undefined): Settings {
  const settings = defaults();
  if (!raw || typeof raw !== "object") return settings;
  if (typeof raw.remember === "boolean") settings.remember = raw.remember;
  if (REPEATS.includes(raw.repeat as string)) settings.repeat = raw.repeat!;
  if (typeof raw.shuffle === "boolean") settings.shuffle = raw.shuffle;
  if (MODES.includes(raw.openMode as string)) settings.openMode = raw.openMode!;
  if (typeof raw.gapless === "boolean") settings.gapless = raw.gapless;
  if (typeof raw.clearOnOpenFolder === "boolean") settings.clearOnOpenFolder = raw.clearOnOpenFolder;
  return settings;
}

/** `playlists.json` and `probe-cache.json` in the plugin's data directory. */
export class Store {
  readonly file: string;
  readonly cacheFile: string;
  constructor(directory: string) {
    this.file = join(directory, "playlists.json");
    this.cacheFile = join(directory, "probe-cache.json");
  }

  /** Read the saved state; a damaged file is kept as `.bad-<time>` and defaults are used. */
  load(): { settings: Settings; list: SavedList | null } {
    if (!existsSync(this.file)) return { settings: defaults(), list: null };
    try {
      const saved = JSON.parse(readFileSync(this.file, "utf8")) as Saved;
      if (saved?.version !== 1) throw new Error("unknown version");
      const list = saved.list && Array.isArray(saved.list.rows) ? saved.list : null;
      return { settings: readSettings(saved.settings), list };
    } catch {
      try {
        renameSync(this.file, `${this.file}.bad-${Date.now()}`);
      } catch {
        /* keep going with defaults */
      }
      return { settings: defaults(), list: null };
    }
  }

  /** Rebuild the queue from a saved list (no file checks; nothing is opened). */
  static restore(queue: Queue, list: SavedList) {
    const rows = list.rows
      .filter(
        (row) =>
          row && (row.kind === "file" || row.kind === "url") && typeof row.source === "string" && row.source,
      )
      .slice(0, ROW_LIMIT);
    for (const row of rows)
      queue.restoreRow({
        kind: row.kind,
        source: row.source,
        named: typeof row.title === "string" ? row.title : null,
        duration: typeof row.duration === "number" && row.duration > 0 ? row.duration : null,
      });
    if (list.sort && SORT_KEYS.includes(list.sort.key)) {
      queue.sortKey = list.sort.key;
      queue.descending = list.sort.descending === true;
    }
    const at = (index: unknown) => (typeof index === "number" ? queue.rows[index]?.rowId : undefined);
    const cursor = at(list.cursor);
    if (cursor) queue.selected = cursor;
    if (Array.isArray(list.selected))
      for (const index of list.selected) {
        const id = at(index);
        if (id) queue.selection.add(id);
      }
  }

  /** The JSON saved for `settings` and (when remembered) the queue. */
  static snapshot(queue: Queue, settings: Settings): string {
    const saved: Saved = { version: 1, settings };
    if (settings.remember) {
      const index = new Map(queue.rows.map((row, i) => [row.rowId, i]));
      saved.list = {
        sort: queue.sortKey ? { key: queue.sortKey, descending: queue.descending } : null,
        cursor: queue.selected ? (index.get(queue.selected) ?? null) : null,
        selected: [...queue.selection].map((id) => index.get(id)).filter((i): i is number => i !== undefined).sort((a, b) => a - b),
        rows: queue.rows.map((row) => ({
          kind: row.kind,
          source: row.source,
          title: row.named,
          duration: lengthOf(row),
        })),
      };
    }
    return JSON.stringify(saved);
  }

  save(json: string) {
    writeAtomic(this.file, json);
  }
  loadCache(): unknown {
    try {
      return JSON.parse(readFileSync(this.cacheFile, "utf8"));
    } catch {
      return [];
    }
  }
  saveCache(entries: unknown) {
    writeAtomic(this.cacheFile, JSON.stringify(entries));
  }
}
