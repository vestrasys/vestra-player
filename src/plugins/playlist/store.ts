import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";
import type { OpenMode } from "./follow.ts";
import { lengthOf, ROW_LIMIT, type Queue, type Repeat, type SortKey } from "./queue.ts";

/** C17 settings, saved even when the list is not remembered. */
export type Settings = {
  remember: boolean;
  repeat: Repeat;
  shuffle: boolean;
  openMode: OpenMode;
  /** B26 "快速推进到下一个" (the key keeps the name of the original gapless item). */
  gapless: boolean;
  /** A03: clear the list before "打开文件夹" adds (no menu entry). */
  clearOnOpenFolder: boolean;
  /** 移除全部 asks first; turned off by the confirmation's 「不再询问」. */
  confirmClear: boolean;
};
type SavedList = {
  sort: { key: Exclude<SortKey, "shuffle" | "reverse">; descending: boolean } | null;
  cursor: number | null;
  selected: number[];
  rows: { kind: "file" | "url"; source: string; title: string | null; duration: number | null }[];
};
export type Saved = { version: 1; settings: Settings; list?: SavedList };

export const defaults = (): Settings => ({
  remember: true,
  repeat: "none",
  shuffle: false,
  openMode: "only",
  gapless: false,
  clearOnOpenFolder: false,
  confirmClear: true,
});

const REPEATS = ["none", "all", "one"];
const MODES = ["only", "similar", "folder"];
const SORT_KEYS = ["title", "filename", "extension", "size", "duration", "date", "folder", "path"];

function readSettings(raw: Partial<Settings> | undefined): Settings {
  const settings = defaults();
  if (!raw || typeof raw !== "object") return settings;
  if (typeof raw.remember === "boolean") settings.remember = raw.remember;
  if (REPEATS.includes(raw.repeat as string)) settings.repeat = raw.repeat!;
  if (typeof raw.shuffle === "boolean") settings.shuffle = raw.shuffle;
  if (MODES.includes(raw.openMode as string)) settings.openMode = raw.openMode!;
  if (typeof raw.gapless === "boolean") settings.gapless = raw.gapless;
  if (typeof raw.clearOnOpenFolder === "boolean") settings.clearOnOpenFolder = raw.clearOnOpenFolder;
  if (typeof raw.confirmClear === "boolean") settings.confirmClear = raw.confirmClear;
  return settings;
}

const isSaved = (value: unknown): value is Saved =>
  !!value && typeof value === "object" && (value as Saved).version === 1;
const isCache = (value: unknown): value is unknown[] => Array.isArray(value);
/** C17: a change is written within a second (the stores debounce). */
export const SAVE_DELAY_MS = 1000;

/**
 * `playlists.json` and `probe-cache.json` in the plugin's data directory
 * (`plugin.storage`): a file that does not parse or has an unknown version is
 * kept as `.bad-<time>` and defaults are used.
 */
export class Store {
  readonly lists: JsonStore<Saved>;
  readonly cache: JsonStore<unknown[]>;
  constructor(scope: Scope) {
    this.lists = scope.storage.json<Saved>("playlists", {
      defaults: { version: 1, settings: defaults() },
      validate: isSaved,
      debounceMs: SAVE_DELAY_MS,
    });
    this.cache = scope.storage.json<unknown[]>("probe-cache", {
      defaults: [],
      validate: isCache,
      debounceMs: SAVE_DELAY_MS,
    });
  }

  /** The saved state, field by field: a bad field falls back to its default. */
  load(): { settings: Settings; list: SavedList | null } {
    const saved = this.lists.value;
    const list = saved.list && Array.isArray(saved.list.rows) ? saved.list : null;
    return { settings: readSettings(saved.settings), list };
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

  /** What is saved for `settings` and (when remembered) the queue. */
  static snapshot(queue: Queue, settings: Settings): Saved {
    const saved: Saved = { version: 1, settings: { ...settings } };
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
    return saved;
  }
}
