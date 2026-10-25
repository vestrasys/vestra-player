import type { JsonStore, Player, PlayerState, Scope } from "@vestra-player/sdk";
import { openStore } from "./settings.ts";

/** The player's device volume range (`player.setOutput`): a louder volume set elsewhere is kept. */
export const MAX_VOLUME = 200;
/** A burst of changes (volume keys held down, the wheel) is written once. */
export const OUTPUT_SAVE_MS = 500;

type Output = { volume: number; muted: boolean };
type Saved = { volume?: unknown; muted?: unknown };

const isObject = (value: unknown): value is Saved =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** A stored volume, clamped to the player's range; anything else is ignored. */
export function storedVolume(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.max(0, Math.min(MAX_VOLUME, value));
}

/**
 * The device volume and mute are kept across restarts in `output.json`. The
 * player owns them for the running session (anyone may change them: the bar,
 * the volume keys, other plugins), so the remembered value is applied only
 * once, at start; after that every change, whoever made it, is adopted and
 * saved. Nothing is saved while the bar's slider is held: the release saves.
 */
export class OutputMemory {
  readonly #store: JsonStore<Saved>;
  readonly #player: Player;
  /** What was last saved (or loaded); `null` until the start is settled. */
  #saved: Output | null = null;
  #held = false;

  constructor(scope: Scope, player: Player) {
    this.#store = openStore<Saved>(scope, "output", { defaults: {}, validate: isObject, debounceMs: OUTPUT_SAVE_MS });
    this.#player = player;
  }

  /** Apply the remembered output once; resolves when later changes are being saved. */
  async restore() {
    const raw = this.#store.value;
    const volume = storedVolume(raw.volume);
    const muted = typeof raw.muted === "boolean" ? raw.muted : null;
    const { state } = this.#player;
    const wanted: { volume?: number; muted?: boolean } = {};
    if (volume !== null && volume !== state.volume) wanted.volume = volume;
    if (muted !== null && muted !== state.muted) wanted.muted = muted;
    // Superseded (someone set the output first) or refused: their value stands.
    if (Object.keys(wanted).length) await this.#player.setOutput(wanted).catch(() => false);
    const now = this.#player.state;
    this.#saved = { volume: volume ?? now.volume, muted: muted ?? now.muted };
    this.observe(now);
  }

  /** The bar's slider is being dragged: the release is saved, not every step. */
  hold(held: boolean) {
    this.#held = held;
    if (!held) this.observe(this.#player.state);
  }

  observe(state: PlayerState) {
    const saved = this.#saved;
    if (!saved || this.#held) return;
    if (state.volume === saved.volume && state.muted === saved.muted) return;
    this.#saved = { volume: state.volume, muted: state.muted };
    this.#store.save({ ...this.#saved });
  }
}
