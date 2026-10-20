import type { PlayerState, Plugin } from "../../sdk/plugin/index.ts";
import type { Preferences } from "./settings.ts";

export const playing = (state: PlayerState) => state.phase === "ready" && !state.paused;

/**
 * H05: the window floats when the mode says so (always, or while playing)
 * or while 迷你尺寸 is on. Only changes are sent, in order, so another
 * plugin's last write is not overridden for nothing; the core drops this
 * plugin's request when it is deactivated.
 */
export class OnTop {
  readonly #plugin: Plugin;
  readonly #preferences: Preferences;
  readonly #mini: () => boolean;
  #applied = false;
  #queue: Promise<void> = Promise.resolve();

  constructor(plugin: Plugin, preferences: Preferences, mini: () => boolean) {
    this.#plugin = plugin;
    this.#preferences = preferences;
    this.#mini = mini;
  }

  get wanted() {
    const mode = this.#preferences.value.onTop;
    return this.#mini() || mode === "always" || (mode === "playing" && playing(this.#plugin.player.state));
  }

  /** Follow the mode, the playback state and 迷你尺寸. */
  apply() {
    const wanted = this.wanted;
    if (wanted === this.#applied) return this.#queue;
    this.#applied = wanted;
    // Headless players refuse it (`unavailable`); there is nothing to float then.
    this.#queue = this.#queue.then(() => this.#plugin.window.setAlwaysOnTop(wanted)).catch(() => {});
    return this.#queue;
  }
}
