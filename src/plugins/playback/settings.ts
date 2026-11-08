import type { JsonStore, Scope } from "@vestra-player/sdk";

export type SeekSteps = [number, number, number, number];

export type Settings = {
  openUrl: { pasteFromClipboard: boolean };
  preventSleep: boolean;
  /** I06 / IA-PB-01: off by default. */
  hoverTime: boolean;
  /** I07: on by default from 0.2.0 (a keyframe sprite made in the background); off before. */
  hoverThumbnail: boolean;
  /**
   * 2: written by 0.2.0 or later. Before, the whole object was saved on any
   * change, so a saved `hoverThumbnail: false` was mostly the old default, not
   * a choice: such a file gets the new default.
   */
  format: number;
  /** B06: seconds of the four relative-seek tiers (← →, Ctrl, Shift, Ctrl+Alt). */
  seekSteps: SeekSteps;
  /**
   * B06: the step seeks of all four tiers land on keyframes ("Jump to keyframes
   * only"); on by default. 0.2.3: the seek bar's clicks and drags follow the
   * same switch (user ruling 2026-10-27); Jump to Time and Jump to Start stay exact.
   */
  seekKeyframes: boolean;
  /** G-14: the bar's current-time label shows the remaining time. */
  remainingTime: boolean;
  /** B23 A-B Loop: Loop Count (`null` = endless; a number = jumps back, the player's `count`). */
  abCount: Count;
};

/** B23 loop count choices, in the Loop Count commands' and the panel's Count order. */
export const COUNTS = [null, 1, 2, 3, 5, 10] as const;
export type Count = (typeof COUNTS)[number];

export const DEFAULT_STEPS: SeekSteps = [5, 30, 60, 300];
/** `Settings.format` of this version. */
export const SETTINGS_FORMAT = 2;

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
  hoverThumbnail: true,
  format: SETTINGS_FORMAT,
  seekSteps: [...DEFAULT_STEPS],
  seekKeyframes: true,
  remainingTime: false,
  abCount: null,
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
 * single bad or missing fields fall back to their defaults. The value is kept
 * in `plugin.state`: the bar, the panel and the commands that read a setting
 * follow its change.
 */
export class SettingsStore {
  readonly #store: JsonStore<Record<string, unknown>>;
  readonly #st: { value: Settings };

  constructor(scope: Scope) {
    this.#store = openStore<Record<string, unknown>>(scope, "settings", { defaults: {}, validate: isObject });
    const raw = this.#store.value;
    const value = defaults();
    const openUrl = raw.openUrl;
    if (isObject(openUrl) && typeof openUrl.pasteFromClipboard === "boolean")
      value.openUrl.pasteFromClipboard = openUrl.pasteFromClipboard;
    if (typeof raw.preventSleep === "boolean") value.preventSleep = raw.preventSleep;
    if (typeof raw.hoverTime === "boolean") value.hoverTime = raw.hoverTime;
    // An older file's `false` was the old default: the new default (on) wins; its `true` stays.
    if (typeof raw.hoverThumbnail === "boolean" && (raw.format === SETTINGS_FORMAT || raw.hoverThumbnail))
      value.hoverThumbnail = raw.hoverThumbnail;
    if (typeof raw.remainingTime === "boolean") value.remainingTime = raw.remainingTime;
    // A wrong length or any value out of range: the whole set falls back.
    const steps = raw.seekSteps;
    if (Array.isArray(steps) && steps.length === 4 && steps.every(validStep))
      value.seekSteps = [...steps] as SeekSteps;
    if (typeof raw.seekKeyframes === "boolean") value.seekKeyframes = raw.seekKeyframes;
    if ((COUNTS as readonly unknown[]).includes(raw.abCount)) value.abCount = raw.abCount as Count;
    this.#st = scope.state({ value });
  }

  get value(): Readonly<Settings> {
    return this.#st.value;
  }

  /** Apply `change` to a copy and save it (written shortly after; a failed write keeps it for this run). */
  update(change: (settings: Settings) => void) {
    const next = structuredClone(this.#st.value);
    change(next);
    this.#st.value = next;
    this.#store.save(next);
  }
}

/** `2.5`, `10`: no trailing zeros. */
export const stepText = (seconds: number) => String(Math.round(seconds * 10) / 10);

/** A typed seek step as seconds, or `null` when empty, not a number, out of range or finer than 0.1. */
export function parseStep(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+(\.\d)?$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return validStep(value) ? value : null;
}
