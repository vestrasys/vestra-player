import type { JsonStore, Scope } from "../../sdk/plugin/index.ts";

/** `<dataDirectory>/visualizer.json` (feature-requirements J11, H12). */
export interface Settings {
  audio: { on: boolean };
  /** `logo`: the custom logo's file name in the data directory (`logo.png` / `logo.jpg`), or null for the brand logo. */
  idle: { on: boolean; logo: string | null };
}

export const DEFAULTS: Settings = { audio: { on: true }, idle: { on: true, logo: null } };

const LOGO_NAME = /^logo\.(png|jpg)$/;

/** A stored value field by field: a bad field falls back to its default, the rest stays. */
export function readSettings(value: unknown): Settings {
  const object = (v: unknown) => (typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : {});
  const root = object(value);
  const audio = object(root.audio);
  const idle = object(root.idle);
  return {
    audio: { on: typeof audio.on === "boolean" ? audio.on : DEFAULTS.audio.on },
    idle: {
      on: typeof idle.on === "boolean" ? idle.on : DEFAULTS.idle.on,
      logo: typeof idle.logo === "string" && LOGO_NAME.test(idle.logo) ? idle.logo : null,
    },
  };
}

/** The settings file; without a data directory the values live for this run only. */
export class SettingsStore {
  readonly #store: JsonStore<unknown> | null;
  #value: Settings;
  constructor(scope: Scope) {
    let store: JsonStore<unknown> | null = null;
    try {
      store = scope.storage.json<unknown>("visualizer", {
        defaults: DEFAULTS,
        validate: (v): v is unknown => typeof v === "object" && v !== null && !Array.isArray(v),
      });
    } catch {
      store = null;
    }
    this.#store = store;
    this.#value = readSettings(store?.value ?? DEFAULTS);
  }
  get value(): Settings {
    return this.#value;
  }
  update(change: (draft: Settings) => void) {
    const next = structuredClone(this.#value);
    change(next);
    this.#value = next;
    this.#store?.save(next);
  }
}
