import type { JsonStore, Scope } from "@vestra-player/sdk";

/** H05 / IA-WIN-03: the four modes, in `OnTopMode` order (0–3), which is also Ctrl+T's cycle. */
export type OnTop = "never" | "always" | "playing" | "fullscreen";
export const ON_TOP: readonly OnTop[] = ["never", "always", "playing", "fullscreen"];

/** G-02: the window size when a source starts playing (`PlayScreenSize`); a scale of the video size, or fit its aspect ratio. */
export type AutoSize = "off" | 0.5 | 1 | 1.5 | 2 | "fit";
export const AUTO_SIZES: readonly AutoSize[] = ["off", 0.5, 1, 1.5, 2, "fit"];

/** `settings.json`: the preferences the core does not keep (H05, B30, G-02). */
export interface Settings {
  onTop: OnTop;
  hideCursorWhilePlaying: boolean;
  autoSize: AutoSize;
  autoSizeOnce: boolean;
}
/** The defaults: `OnTopMode=3`, `AutoHideMouse=1`, `PlayScreenSize=2` (1.0x), `PlayScreenSizeFirst=0`. */
export const DEFAULTS: Settings = { onTop: "fullscreen", hideCursorWhilePlaying: true, autoSize: 1, autoSizeOnce: false };

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** After a source becomes ready, `stats()` is asked again this often until it reports the video size. */
  probeRetry: 500,
  /** …at most this many times. */
  probeAttempts: 10,
};

const isObject = (value: unknown): value is Settings =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const bool = (value: unknown, fallback: boolean) => (typeof value === "boolean" ? value : fallback);

/** One bad field falls back to its default; the others are kept. */
export function normalize(value: Partial<Record<keyof Settings, unknown>>): Settings {
  return {
    onTop: ON_TOP.includes(value.onTop as OnTop) ? (value.onTop as OnTop) : DEFAULTS.onTop,
    hideCursorWhilePlaying: bool(value.hideCursorWhilePlaying, DEFAULTS.hideCursorWhilePlaying),
    autoSize: AUTO_SIZES.includes(value.autoSize as AutoSize) ? (value.autoSize as AutoSize) : DEFAULTS.autoSize,
    autoSizeOnce: bool(value.autoSizeOnce, DEFAULTS.autoSizeOnce),
  };
}

/** The settings and a typed save; without a data directory they live for this run only. */
export class Preferences {
  readonly #store: JsonStore<Settings> | null;
  #value: Settings;
  constructor(scope: Scope) {
    let store: JsonStore<Settings> | null = null;
    try {
      store = scope.storage.json<Settings>("settings", { defaults: DEFAULTS, validate: isObject });
    } catch {
      store = null;
    }
    this.#store = store;
    this.#value = normalize(store?.value ?? DEFAULTS);
  }
  get value(): Settings {
    return this.#value;
  }
  update(patch: Partial<Settings>) {
    this.#value = { ...this.#value, ...patch };
    this.#store?.save(this.#value);
  }
}
