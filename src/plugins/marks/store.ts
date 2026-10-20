import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** B21: bookmarks.json is written this long after the last change. */
  bookmarksWrite: 500,
  /** marks.json (loop count) is written this long after the last change. */
  settingsWrite: 300,
  /** B21: rebuilds of the dynamic chapter/bookmark menu within this window are merged. */
  menuRebuild: 150,
  /** B23: nudges (held keys repeat) are committed to the player this long after the last one. */
  loopCommit: 100,
};

const isObject = <T>(value: unknown): value is T =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * `<dataDirectory>/<name>.json` through `plugin.storage.json`; the fields are
 * checked by the reader, so one bad field never drops the rest. Without a
 * data directory the values live for this run only.
 */
export function jsonStore<T>(plugin: Scope, name: string, defaults: T, debounceMs: number): JsonStore<T> {
  try {
    return plugin.storage.json<T>(name, { defaults, validate: isObject<T>, debounceMs });
  } catch {
    const memory = {
      value: defaults,
      save: (next: T) => void (memory.value = next),
      flush: async () => {},
      dispose: async () => {},
    };
    return memory;
  }
}

/** B23 loop count choices: `null` = endless; a number = jumps back (the player's `count`). */
export const COUNTS = [null, 1, 2, 3, 5, 10] as const;
export type Count = (typeof COUNTS)[number];

type SavedSettings = { version: 1; abCount: Count };

/** marks.json: `{ version: 1, abCount }` (B22's `showOnSeekbar` joins it in round 3b). */
export class Settings {
  readonly #store: JsonStore<SavedSettings>;
  constructor(plugin: Scope) {
    this.#store = jsonStore<SavedSettings>(plugin, "marks", { version: 1, abCount: null }, timing.settingsWrite);
  }
  get abCount(): Count {
    const value = (this.#store.value as { abCount?: unknown }).abCount;
    return (COUNTS as readonly unknown[]).includes(value) ? (value as Count) : null;
  }
  setAbCount(count: Count) {
    this.#store.save({ version: 1, abCount: count });
  }
}
