import type { BindingRef, JsonStore, Scope } from "../../sdk/plugin/index.ts";

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** `bindings.onChange` bursts are merged into one `list()` this long after the last. */
  reload: 100,
  /** hotkeys.json / ignore-defaults.json are written this long after the last change. */
  write: 300,
};

const isObject = <T>(value: unknown): value is T =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * `<dataDirectory>/<name>.json` through `plugin.storage.json`; fields are
 * checked by the reader. Without a data directory the values live for this
 * run only.
 */
export function jsonStore<T>(plugin: Scope, name: string, defaults: T): JsonStore<T> {
  try {
    return plugin.storage.json<T>(name, { defaults, validate: isObject<T>, debounceMs: timing.write });
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

/** J01 list preferences (hotkeys.json): sort column and filters. */
export type Prefs = {
  version: 1;
  sort: "command" | "keys";
  descending: boolean;
  /** 0 全部, 1 已修改, 2 有冲突, 3 未绑定. */
  status: number;
  /** `""` all sources, `"host"`, a plugin id, or `"*orphans"` (未加载的插件). */
  source: string;
};
export const PREFS: Prefs = { version: 1, sort: "command", descending: false, status: 0, source: "" };

export function readPrefs(value: Partial<Prefs>): Prefs {
  return {
    version: 1,
    sort: value.sort === "keys" ? "keys" : "command",
    descending: value.descending === true,
    status: Number.isInteger(value.status) && value.status! >= 0 && value.status! <= 3 ? value.status! : 0,
    source: typeof value.source === "string" ? value.source : "",
  };
}

/** J03 record (ignore-defaults.json, ruling R-K2): the switch and the overrides it wrote. */
export type IgnoreRecord = { version: 1; enabled: boolean; written: BindingRef[] };

export function readRecord(value: Partial<IgnoreRecord>): IgnoreRecord {
  const written = Array.isArray(value.written)
    ? value.written.filter(
        (ref): ref is BindingRef =>
          typeof ref?.plugin === "string" && typeof ref.command === "string" && !!ref.plugin && !!ref.command,
      )
    : [];
  return { version: 1, enabled: value.enabled === true, written };
}
