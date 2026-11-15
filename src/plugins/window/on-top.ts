import type { PlayerState, Plugin } from "@vestra-player/sdk";
import type { Preferences } from "./settings.ts";

export const playing = (state: PlayerState) => state.phase === "ready" && !state.paused;

/**
 * H05 / IA-WIN-03: the window floats when the mode says so (by default never): always, while
 * playing, or while playing in fullscreen. Mini Size
 * does not float (IA-WIN-02). Only changes are sent, in order, so another
 * plugin's last write is not overridden for nothing; the core drops this
 * plugin's request when it is deactivated. Callers re-apply on player and
 * window changes.
 */
export class OnTop {
  readonly #plugin: Plugin;
  readonly #preferences: Preferences;
  #applied = false;
  #queue: Promise<void> = Promise.resolve();

  constructor(plugin: Plugin, preferences: Preferences) {
    this.#plugin = plugin;
    this.#preferences = preferences;
  }

  get wanted() {
    const { player, window } = this.#plugin;
    switch (this.#preferences.value.onTop) {
      case "always":
        return true;
      case "playing":
        return playing(player.state);
      case "fullscreen":
        return window.state.fullscreen && playing(player.state);
      default:
        return false;
    }
  }

  /** Follow the mode, the playback state and fullscreen. */
  apply() {
    const wanted = this.wanted;
    if (wanted === this.#applied) return this.#queue;
    this.#applied = wanted;
    // Headless players refuse it (`unavailable`); there is nothing to float then.
    this.#queue = this.#queue.then(() => this.#plugin.window.setAlwaysOnTop(wanted)).catch(() => {});
    return this.#queue;
  }
}
