import type { PlaybackStats, PlayerState, Plugin } from "../../sdk/plugin/index.ts";
import type { Size } from "./geometry.ts";
import { timing } from "./settings.ts";

/** The display size `stats()` reports (aspect corrected, after filters), else the decoded size. */
export function displaySize(video: PlaybackStats["video"]): Size | null {
  const width = video?.displayWidth ?? video?.width ?? null;
  const height = video?.displayHeight ?? video?.height ?? null;
  return width && height && width > 0 && height > 0 ? { width, height } : null;
}

const showing = (state: PlayerState) => state.phase === "ready" || state.phase === "ended";

/**
 * The current source's video display size (H03, H04). `PlayerState` has no
 * video size, so it is read with `player.stats()` when a source becomes
 * ready (asked again a few times while mpv has not decided it yet) and
 * before every resize. `null`: no source, or no video (audio only).
 */
export class VideoSize {
  readonly #plugin: Plugin;
  #size: Size | null = null;
  #source = -1;
  #probed = -1;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #disposed = false;
  /** G-02: called once per source, when the probe after its first `ready` finds the video size. */
  onFirstSize: ((source: number, size: Size) => void) | null = null;

  constructor(plugin: Plugin) {
    this.#plugin = plugin;
    plugin.onDispose(() => {
      this.#disposed = true;
      if (this.#timer) clearTimeout(this.#timer);
    });
  }

  get size() {
    return this.#size;
  }

  observe(state: PlayerState) {
    if (state.sourceId !== this.#source || !showing(state)) {
      if (this.#size) this.#plugin.update();
      this.#size = null;
      this.#source = state.sourceId;
    }
    if (state.phase === "ready" && this.#probed !== state.sourceId) {
      this.#probed = state.sourceId;
      if (this.#timer) clearTimeout(this.#timer);
      void this.#probe(state.sourceId, timing.probeAttempts);
    }
  }

  /** Read the size now (a filter may have rotated the picture since). */
  async refresh(): Promise<Size | null> {
    const source = this.#plugin.player.state.sourceId;
    const stats = await this.#plugin.player.stats();
    const current = this.#plugin.player.state;
    if (current.sourceId !== source || !showing(current)) return this.#size;
    this.#size = displaySize(stats.video);
    return this.#size;
  }

  async #probe(source: number, attempts: number) {
    this.#timer = null;
    const stats = await this.#plugin.player.stats().catch(() => null);
    const current = this.#plugin.player.state;
    if (this.#disposed || current.sourceId !== source || !showing(current)) return;
    const size = displaySize(stats?.video ?? null);
    if (size) {
      this.#size = size;
      this.#plugin.update();
      this.onFirstSize?.(source, size);
    } else if (attempts > 1) {
      this.#timer = setTimeout(() => void this.#probe(source, attempts - 1), timing.probeRetry);
    }
  }
}
