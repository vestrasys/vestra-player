import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";

export type OnTop = "never" | "always" | "playing";
export const ON_TOP: readonly OnTop[] = ["never", "always", "playing"];

/** `settings.json`: the two preferences the core does not keep (H05, B30). */
export interface Settings {
  onTop: OnTop;
  hideCursorWhilePlaying: boolean;
}
export const DEFAULTS: Settings = { onTop: "never", hideCursorWhilePlaying: false };

/** Delays, in milliseconds; tests shorten them. */
export const timing = {
  /** After a source becomes ready, `stats()` is asked again this often until it reports the video size. */
  probeRetry: 500,
  /** …at most this many times. */
  probeAttempts: 10,
};

const isObject = (value: unknown): value is Settings =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** One bad field falls back to its default; the others are kept. */
export function normalize(value: Partial<Record<keyof Settings, unknown>>): Settings {
  return {
    onTop: ON_TOP.includes(value.onTop as OnTop) ? (value.onTop as OnTop) : DEFAULTS.onTop,
    hideCursorWhilePlaying:
      typeof value.hideCursorWhilePlaying === "boolean" ? value.hideCursorWhilePlaying : DEFAULTS.hideCursorWhilePlaying,
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
