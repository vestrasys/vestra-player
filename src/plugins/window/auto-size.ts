import type { Plugin, WindowState } from "@vestra-player/sdk";
import type { Size } from "./geometry.ts";
import type { PictureInPicture } from "./pip.ts";
import type { Preferences } from "./settings.ts";
import type { Sizer } from "./size.ts";
import type { VideoSize } from "./video-size.ts";

/** `storage.session` key: the last source seen and whether this run saw a video source (Only Once). */
const SEEN = "autoSize.seen";
interface Seen {
  source: number;
  seenVideo: boolean;
}
const isSeen = (value: unknown): value is Seen =>
  typeof value === "object" &&
  value !== null &&
  typeof (value as Seen).source === "number" &&
  typeof (value as Seen).seenVideo === "boolean";

/**
 * G-02 window size when playback starts (default 1.0x): each
 * new source (`sourceId`) is sized once, when its video size first becomes
 * known. Not when the source has no video, in fullscreen, maximized (as
 * MPC-HC's auto-zoom and mpv's autofit leave a maximized window alone; the
 * size commands still restore and resize it), with Mini Size on,
 * in picture-in-picture, or when the user dragged the window while it played (the size
 * commands stay disabled until the size is known); with Only Once, only the
 * run's first video source. Replaying the same source, pausing and seeking do not trigger it
 * (the size is probed once per source). A new source is told by `source.opening`; the last
 * source seen is kept in session storage, so a plugin started again after standby does not
 * size the source that was already playing.
 */
export class AutoSize {
  readonly #plugin: Plugin;
  readonly #preferences: Preferences;
  readonly #sizer: Sizer;
  readonly #pip: PictureInPicture;
  readonly #video: VideoSize;
  /** The source waiting for its size, `null` once it was handled or given up. */
  #pending: { source: number } | null = null;
  #source = -1;
  /** A video source was seen in this run (Only Once). */
  #seenVideo = false;
  #window: { fullscreen: boolean; area: Size };

  constructor(plugin: Plugin, preferences: Preferences, sizer: Sizer, pip: PictureInPicture, video: VideoSize) {
    this.#plugin = plugin;
    this.#preferences = preferences;
    this.#sizer = sizer;
    this.#pip = pip;
    this.#video = video;
    this.#window = this.#snapshot(plugin.window.state);
    plugin.player.on("window.changed", (state) => this.#windowChanged(state));
    plugin.player.on("source.opening", ({ state }) => this.#observe(state.sourceId));
  }

  /**
   * At activation, with the state read once: the current source waits for its size unless this
   * player run already saw it (the plugin was started again after standby).
   */
  async start(sourceId: number) {
    const saved = await this.#plugin.storage.session.get<unknown>(SEEN).catch(() => undefined);
    if (isSeen(saved)) this.#seenVideo ||= saved.seenVideo;
    if (this.#source !== -1) return; // a `source.opening` came first
    if (isSeen(saved) && saved.source === sourceId) this.#source = sourceId;
    else this.#observe(sourceId);
  }

  /** A new source waits for its size. */
  #observe(sourceId: number) {
    if (sourceId === this.#source) return;
    this.#source = sourceId;
    this.#pending = { source: sourceId };
    this.#save();
  }

  #save() {
    const seen: Seen = { source: this.#source, seenVideo: this.#seenVideo };
    void this.#plugin.storage.session.set(SEEN, seen).catch(() => {});
  }

  /** The source's video size is known (once per source, from `VideoSize`). */
  async sized(source: number, video: Size) {
    const pending = this.#pending;
    if (!pending || pending.source !== source) return;
    this.#pending = null;
    const first = !this.#seenVideo;
    this.#seenVideo = true;
    if (first) this.#save();
    const { autoSize, autoSizeOnce } = this.#preferences.value;
    if (autoSize === "off" || (autoSizeOnce && !first)) return;
    const { state } = this.#plugin.window;
    // `maximized` reads false on players before 0.6.3, which do not report it.
    if (state.fullscreen || state.maximized || state.pictureInPicture.active || this.#pip.switching) return;
    if (this.#sizer.mini || this.#sizer.resizing) return;
    await this.#sizer.auto(autoSize, video);
  }

  /** The user resized the window while the source played and before its size was known: leave it. */
  #windowChanged(state: WindowState) {
    const before = this.#window;
    this.#window = this.#snapshot(state);
    if (!this.#pending || this.#sizer.resizing) return;
    if (!this.#video.ready || before.fullscreen || state.fullscreen || state.pictureInPicture.active) return;
    const moved = Math.abs(before.area.width - state.videoArea.width) > 1 || Math.abs(before.area.height - state.videoArea.height) > 1;
    if (moved) this.#pending = null;
  }

  #snapshot(state: WindowState) {
    return { fullscreen: state.fullscreen, area: { width: state.videoArea.width, height: state.videoArea.height } };
  }
}
