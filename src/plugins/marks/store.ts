import type { JsonStore, Scope } from "@vestra-player/sdk";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** B21: bookmarks.json is written this long after the last change. */
  bookmarksWrite: 500,
  /** marks.json (the seek-bar switch) is written this long after the last change. */
  settingsWrite: 300,
  /** B21: rebuilds of the dynamic chapter/bookmark menu within this window are merged. */
  menuRebuild: 150,
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

type SavedSettings = { version: 1; showOnSeekbar: boolean };

/**
 * marks.json: `{ version: 1, showOnSeekbar }` (B22 seek-bar switch). A file
 * from before A-B Loop moved to the playback plugin may also hold `abCount`
 * (its Loop Count); it is kept as it is when the switch is saved.
 */
export class Settings {
  readonly #store: JsonStore<SavedSettings>;
  constructor(plugin: Scope) {
    this.#store = jsonStore<SavedSettings>(plugin, "marks", { version: 1, showOnSeekbar: true }, timing.settingsWrite);
  }
  /** B22 Show Marks on Seek Bar: on unless saved as `false`. */
  get showOnSeekbar(): boolean {
    return (this.#store.value as { showOnSeekbar?: unknown }).showOnSeekbar !== false;
  }
  setShowOnSeekbar(show: boolean) {
    this.#store.save({ ...this.#store.value, version: 1, showOnSeekbar: show });
  }
}
