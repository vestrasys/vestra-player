import type { Plugin, WindowState } from "@vestra-player/sdk";
import type { Size } from "./geometry.ts";
import type { PictureInPicture } from "./pip.ts";
import type { Preferences } from "./settings.ts";
import type { Sizer } from "./size.ts";

/**
 * G-02 开始播放时的窗口尺寸 (reference player `PlayScreenSize`, default 1.0x): each
 * new source (`sourceId`) is sized once, when its video size first becomes
 * known. Not when the source has no video, in fullscreen, with 迷你尺寸 on,
 * in 画中画, or when the user dragged the window while it played (the size
 * commands stay disabled until the size is known); with 只调整一次, only the
 * run's first video source. Replaying the same source, pausing and seeking do not trigger it
 * (the size is probed once per source).
 */
export class AutoSize {
  readonly #plugin: Plugin;
  readonly #preferences: Preferences;
  readonly #sizer: Sizer;
  readonly #pip: PictureInPicture;
  /** The source waiting for its size, `null` once it was handled or given up. */
  #pending: { source: number } | null = null;
  #source = -1;
  /** A video source was seen in this run (只调整一次). */
  #seenVideo = false;
  #window: { fullscreen: boolean; area: Size };

  constructor(plugin: Plugin, preferences: Preferences, sizer: Sizer, pip: PictureInPicture) {
    this.#plugin = plugin;
    this.#preferences = preferences;
    this.#sizer = sizer;
    this.#pip = pip;
    this.#window = this.#snapshot(plugin.window.state);
    plugin.window.onChange((state) => this.#windowChanged(state));
  }

  /** Follow the player: a new source waits for its size. */
  observe(sourceId: number) {
    if (sourceId === this.#source) return;
    this.#source = sourceId;
    this.#pending = { source: sourceId };
  }

  /** The source's video size is known (once per source, from `VideoSize`). */
  async sized(source: number, video: Size) {
    const pending = this.#pending;
    if (!pending || pending.source !== source) return;
    this.#pending = null;
    const first = !this.#seenVideo;
    this.#seenVideo = true;
    const { autoSize, autoSizeOnce } = this.#preferences.value;
    if (autoSize === "off" || (autoSizeOnce && !first)) return;
    const { state } = this.#plugin.window;
    if (state.fullscreen || state.pictureInPicture.active || this.#pip.switching) return;
    if (this.#sizer.mini || this.#sizer.resizing) return;
    await this.#sizer.auto(autoSize, video);
  }

  /** The user resized the window while the source played and before its size was known: leave it. */
  #windowChanged(state: WindowState) {
    const before = this.#window;
    this.#window = this.#snapshot(state);
    if (!this.#pending || this.#sizer.resizing) return;
    const { phase } = this.#plugin.player.state;
    if (phase !== "ready" || before.fullscreen || state.fullscreen || state.pictureInPicture.active) return;
    const moved = Math.abs(before.area.width - state.videoArea.width) > 1 || Math.abs(before.area.height - state.videoArea.height) > 1;
    if (moved) this.#pending = null;
  }

  #snapshot(state: WindowState) {
    return { fullscreen: state.fullscreen, area: { width: state.videoArea.width, height: state.videoArea.height } };
  }
}
