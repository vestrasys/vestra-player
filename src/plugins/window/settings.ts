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
  /** The user chose `onTop` (menu, Ctrl+T, palette, settings page); otherwise it follows the default. */
  onTopChosen: boolean;
  hideCursorWhilePlaying: boolean;
  autoSize: AutoSize;
  autoSizeOnce: boolean;
}
/**
 * The defaults: on top Never (IA-WIN-03, user ruling 2026-11-15: mpv's `ontop=no`, VLC's "Never"),
 * `AutoHideMouse=1`, `PlayScreenSize=2` (1.0x), `PlayScreenSizeFirst=0`.
 */
export const DEFAULTS: Settings = {
  onTop: "never",
  onTopChosen: false,
  hideCursorWhilePlaying: true,
  autoSize: 1,
  autoSizeOnce: false,
};
/** The default up to 0.1.8 (PotPlayer's `OnTopMode=3`). */
const OLD_DEFAULT_ON_TOP: OnTop = "fullscreen";

/** Runs `run` after `ms` milliseconds; the result cancels it. */
export type Later = (run: () => void, ms: number) => () => void;

/** Delays, in milliseconds, and the clock that waits them; tests replace them. */
export const timing: { probeRetry: number; probeAttempts: number; later: Later } = {
  /** After a source becomes ready, `stats()` is asked again this often until it reports the video size. */
  probeRetry: 500,
  /** …at most this many times. */
  probeAttempts: 10,
  later: (run, ms) => {
    const timer = setTimeout(run, ms);
    return () => clearTimeout(timer);
  },
};

const isObject = (value: unknown): value is Settings =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const bool = (value: unknown, fallback: boolean) => (typeof value === "boolean" ? value : fallback);

/**
 * Whether the stored `onTop` is the user's choice. A file from 0.1.9 on says so (`onTopChosen`). An
 * older file holds the whole settings object, written whenever any setting changed, so its `onTop` is
 * a choice only when it differs from the old default; the old default itself is taken as untouched
 * (IA-WIN-03 item 5).
 */
function chosen(value: Partial<Record<keyof Settings, unknown>>): boolean {
  if (!ON_TOP.includes(value.onTop as OnTop)) return false;
  if (typeof value.onTopChosen === "boolean") return value.onTopChosen;
  return value.onTop !== OLD_DEFAULT_ON_TOP;
}

/** One bad field falls back to its default; the others are kept. An on-top mode never chosen follows the default. */
export function normalize(value: Partial<Record<keyof Settings, unknown>>): Settings {
  const onTopChosen = chosen(value);
  return {
    onTop: onTopChosen ? (value.onTop as OnTop) : DEFAULTS.onTop,
    onTopChosen,
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
