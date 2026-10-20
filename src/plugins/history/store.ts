import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** A12: recent.json is written this long after the last change (changes meanwhile are merged). */
  recentWrite: 1000,
  /** A12: menu rebuilds within this window are merged. */
  menuRebuild: 500,
  /** B25: positions.json is written this long after the last change. */
  positionsWrite: 300,
  /** B25: the playing position is remembered this often. */
  positionTick: 5000,
  /** B25: the 每次询问 window closes itself after this long. */
  askTimeout: 10000,
};

/** Any JSON object: the fields are checked one by one by the reader, so one bad field never drops the rest. */
const isObject = <T>(value: unknown): value is T => typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * `<dataDirectory>/<name>.json` through `plugin.storage.json`. Without a data
 * directory the values live for this run only.
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
