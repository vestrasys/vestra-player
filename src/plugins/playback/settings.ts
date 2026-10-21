import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";

export type SeekSteps = [number, number, number, number];

export type Settings = {
  openUrl: { pasteFromClipboard: boolean };
  preventSleep: boolean;
  /** I06 / IA-PB-01: off by default (reference player `PreviewSeekTime=0`). */
  hoverTime: boolean;
  /** I07: off by default (needs ffmpeg and takes CPU). */
  hoverThumbnail: boolean;
  /** B06: seconds of the four relative-seek tiers (← →, Ctrl, Shift, Ctrl+Alt). */
  seekSteps: SeekSteps;
  /** G-14: the bar's current-time label shows the remaining time (reference player `SkinCurrentLeftTime`). */
  remainingTime: boolean;
};

export const DEFAULT_STEPS: SeekSteps = [5, 30, 60, 300];

/** 0.1–3600 seconds with at most one decimal. */
export const validStep = (value: unknown): value is number =>
  typeof value === "number" &&
  Number.isFinite(value) &&
  value >= 0.1 &&
  value <= 3600 &&
  Math.abs(Math.round(value * 10) - value * 10) < 1e-9;

const defaults = (): Settings => ({
  openUrl: { pasteFromClipboard: true },
  preventSleep: true,
  hoverTime: false,
  hoverThumbnail: false,
  seekSteps: [...DEFAULT_STEPS],
  remainingTime: false,
});

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * `name.json` in the plugin's data directory (`plugin.storage.json`). Without a
 * data directory the value lives for this run only.
 */
export function openStore<T>(
  scope: Scope,
  name: string,
  options: { defaults: T; validate?: (value: unknown) => value is T; debounceMs?: number },
): JsonStore<T> {
  try {
    return scope.storage.json(name, options);
  } catch {
    let value = options.defaults;
    return {
      get value() {
        return value;
      },
      save: (next) => void (value = next),
      flush: async () => {},
      dispose: async () => {},
    };
  }
}

/**
 * `settings.json`: a file that is not an object is set aside by the store;
 * single bad or missing fields fall back to their defaults.
 */
export class SettingsStore {
  readonly #store: JsonStore<Record<string, unknown>>;
  #value: Settings;

  constructor(scope: Scope) {
    this.#store = openStore<Record<string, unknown>>(scope, "settings", { defaults: {}, validate: isObject });
    const raw = this.#store.value;
    this.#value = defaults();
    const openUrl = raw.openUrl;
    if (isObject(openUrl) && typeof openUrl.pasteFromClipboard === "boolean")
      this.#value.openUrl.pasteFromClipboard = openUrl.pasteFromClipboard;
    if (typeof raw.preventSleep === "boolean") this.#value.preventSleep = raw.preventSleep;
    if (typeof raw.hoverTime === "boolean") this.#value.hoverTime = raw.hoverTime;
    if (typeof raw.hoverThumbnail === "boolean") this.#value.hoverThumbnail = raw.hoverThumbnail;
    if (typeof raw.remainingTime === "boolean") this.#value.remainingTime = raw.remainingTime;
    // A wrong length or any value out of range: the whole set falls back.
    const steps = raw.seekSteps;
    if (Array.isArray(steps) && steps.length === 4 && steps.every(validStep))
      this.#value.seekSteps = [...steps] as SeekSteps;
  }

  get value(): Readonly<Settings> {
    return this.#value;
  }

  /** Apply `change` to a copy and save it (written shortly after; a failed write keeps it for this run). */
  update(change: (settings: Settings) => void) {
    const next = structuredClone(this.#value);
    change(next);
    this.#value = next;
    this.#store.save(next);
  }
}
