import type { FactEventPayloads, PlaybackStats, PlayerState, Plugin } from "@vestra-player/sdk";
import type { Size } from "./geometry.ts";
import { timing } from "./settings.ts";

/** The display size `stats()` reports (aspect corrected, after filters), else the decoded size. */
export function displaySize(video: PlaybackStats["video"]): Size | null {
  const width = video?.displayWidth ?? video?.width ?? null;
  const height = video?.displayHeight ?? video?.height ?? null;
  return width && height && width > 0 && height > 0 ? { width, height } : null;
}

/** The shown size `video.reconfigured` carries (0 while unknown), else the decoded size. */
function reconfiguredSize(video: FactEventPayloads["video.reconfigured"]): Size | null {
  const width = video.displayWidth > 0 ? video.displayWidth : video.width;
  const height = video.displayHeight > 0 ? video.displayHeight : video.height;
  return width > 0 && height > 0 ? { width, height } : null;
}

/** Where the current source is, as the source events said: `showing` is `ready` or `ended`. */
type Phase = "none" | "loading" | "ready" | "ended";

/**
 * The current source's video display size (H03, H04). `PlayerState` has no
 * video size: `video.reconfigured` brings it, and `player.stats()` is read when a
 * source becomes ready (asked again a few times while mpv has not decided it yet)
 * and before every resize. `null`: no source, or no video (audio only).
 * Whether a source shows (phase `ready` or `ended`) is a trigger the player keeps
 * (it leaves while a source loads, also a restart of the same one), the source and
 * its end follow the source events; nothing here keeps the plugin running.
 */
export class VideoSize {
  readonly #plugin: Plugin;
  #size: Size | null = null;
  #source = -1;
  #phase: Phase = "none";
  /** The source whose size is known or being probed; reset whenever the size is dropped. */
  #probed = -1;
  #announced = -1;
  /** Cancels the pending retry of the probe, if any. */
  #retry: (() => void) | null = null;
  #disposed = false;
  /** What arrived before `start()`: handled after the state read once, in order. */
  #early: (() => void)[] | null = [];
  /** G-02: called once per source, when its video size is first found (not again after a restart). */
  onFirstSize: ((source: number, size: Size) => void) | null = null;

  /** Called in the synchronous part of `activate()`: it declares the trigger. */
  constructor(plugin: Plugin) {
    this.#plugin = plugin;
    plugin.onDispose(() => {
      this.#disposed = true;
      this.#cancelRetry();
    });
    const { player, when } = plugin;
    const after = <T>(handle: (data: T) => void) => (data: T) => {
      if (this.#early) this.#early.push(() => handle(data));
      else handle(data);
    };
    plugin
      .trigger()
      .firesOn(when.phase("ready", "ended"))
      .enter(after(({ state }) => this.#shown(state)))
      .leave(after(({ state }) => this.#drop(state.sourceId, state.phase === "loading" ? "loading" : "none")));
    player.on(
      "source.opening",
      after(({ state }) => this.#drop(state.sourceId, "loading")),
    );
    player.on(
      "source.ready",
      after(({ state }) => this.#shown(state)),
    );
    player.on(
      "source.ended",
      after(({ state }) => {
        // The last frame stays at the end of the file; any other end leaves the trigger.
        if (state.sourceId === this.#source && state.phase === "ended") this.#phase = "ended";
      }),
    );
    player.on(
      "video.reconfigured",
      after((video) => this.#reconfigured(video)),
    );
  }

  get size() {
    return this.#size;
  }

  /** The source is loaded and shows a picture or the last frame (`ready` or `ended`). */
  get showing() {
    return this.#phase === "ready" || this.#phase === "ended";
  }

  /** The source is ready (not ended). */
  get ready() {
    return this.#phase === "ready";
  }

  /** At activation: the state read once; then what arrived meanwhile, and the events from here. */
  start(state: PlayerState) {
    this.#source = state.sourceId;
    this.#phase = state.phase === "loading" ? "loading" : "none";
    if (state.phase === "ready" || state.phase === "ended") this.#shown(state);
    const early = this.#early ?? [];
    this.#early = null;
    for (const handle of early) handle();
  }

  /** Read the size now (a filter may have rotated the picture since). */
  async refresh(): Promise<Size | null> {
    const source = this.#source;
    const stats = await this.#plugin.player.stats();
    if (this.#source !== source || !this.showing) return this.#size;
    this.#size = displaySize(stats.video);
    if (!this.#size) this.#probed = -1; // the next `ready` asks again
    return this.#size;
  }

  /** A new source (or the same one restarted, through loading), or one that left: no size. */
  #drop(source: number, phase: Phase) {
    if (this.#size) this.#plugin.update();
    this.#size = null;
    this.#source = source;
    this.#phase = phase;
    // A restart keeps the source id but passes through `loading`: probe again on its `ready`.
    this.#probed = -1;
    this.#cancelRetry();
  }

  /** The source shows (`ready`, or straight at its end): probed once per ready source. */
  #shown(state: PlayerState) {
    if (state.phase !== "ready" && state.phase !== "ended") return;
    const source = state.sourceId;
    if (source !== this.#source) this.#drop(source, state.phase);
    this.#phase = state.phase;
    if (state.phase !== "ready" || this.#probed === source) return;
    this.#probed = source;
    this.#cancelRetry();
    void this.#probe(source, timing.probeAttempts);
  }

  #reconfigured(video: FactEventPayloads["video.reconfigured"]) {
    if (this.#disposed || video.sourceId !== this.#source || !this.showing) return;
    const size = reconfiguredSize(video);
    if (size) this.#found(video.sourceId, size);
  }

  #found(source: number, size: Size) {
    this.#probed = source;
    this.#cancelRetry();
    this.#size = size;
    this.#plugin.update();
    if (this.#announced !== source) {
      this.#announced = source;
      this.onFirstSize?.(source, size);
    }
  }

  #cancelRetry() {
    this.#retry?.();
    this.#retry = null;
  }

  async #probe(source: number, attempts: number) {
    this.#retry = null;
    const stats = await this.#plugin.player.stats().catch(() => null);
    if (this.#disposed || this.#source !== source || !this.showing) return;
    const size = displaySize(stats?.video ?? null);
    if (size) this.#found(source, size);
    else if (attempts > 1) this.#retry = timing.later(() => void this.#probe(source, attempts - 1), timing.probeRetry);
  }
}
