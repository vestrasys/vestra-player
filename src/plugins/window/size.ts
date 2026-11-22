import type { Plugin } from "@vestra-player/sdk";
import { clampNote, fitted, mini, near, scaled, type Size } from "./geometry.ts";
import { quiet } from "./osd.ts";
import type { AutoSize } from "./settings.ts";
import type { VideoSize } from "./video-size.ts";

/** Why the size commands are disabled, `null` when they can run. */
export function sizeReason(plugin: Plugin, video: VideoSize): "Not available in fullscreen" | "No video" | null {
  if (plugin.window.state.fullscreen) return "Not available in fullscreen";
  if (!video.size) return "No video";
  return null;
}

/**
 * H03 scales, H04 Fit to Video Aspect Ratio, Mini Size (IA-WIN-01) and G-02's
 * adjustment when playback starts: every resize goes through
 * `setVideoArea`, one at a time (`busy` meanwhile).
 * Mini Size remembers the video area before it and ends by itself when the
 * window is resized or goes fullscreen; it does not touch on-top (IA-WIN-02).
 */
export class Sizer {
  readonly #plugin: Plugin;
  readonly #video: VideoSize;
  resizing = false;
  /** While Mini Size is on: the video area to go back to, and the one it set. */
  mini: { previous: Size; applied: Size } | null = null;

  constructor(plugin: Plugin, video: VideoSize) {
    this.#plugin = plugin;
    this.#video = video;
    // Once per change: a resize when the gesture ends, a fullscreen flip at once.
    plugin.player.on("window.changed", (state) => {
      if (!this.mini || this.resizing) return;
      if (state.fullscreen || !near(state.videoArea, this.mini.applied)) this.#endMini();
    });
  }

  /** H03: `scale` times the video's display size. */
  async scale(scale: number) {
    const video = await this.#needVideo();
    if (!video) return;
    const target = scaled(video, scale);
    const applied = await this.#resize(target, true);
    if (!applied) return;
    const note = clampNote(target, applied);
    quiet(
      this.#plugin.osd.show(
        note === "shrunk"
          ? "Window: {}× (shrunk to fit the screen)"
          : note === "enlarged"
            ? "Window: {}× (enlarged to the minimum size)"
            : "Window: {}×",
        [scale],
      ),
    );
  }

  /** H04: same width, the video's aspect ratio. */
  async fit() {
    const video = await this.#needVideo();
    if (!video) return;
    const area = this.#plugin.window.state.videoArea;
    const target = fitted(area, video);
    if (near(area, target)) {
      quiet(this.#plugin.osd.show("The window already fits the video aspect ratio"));
      return;
    }
    const applied = await this.#resize(target, true);
    if (!applied) return;
    const note = clampNote(target, applied);
    quiet(
      this.#plugin.osd.show(
        note === "shrunk"
          ? "Window: Fit to video aspect ratio (shrunk to fit the screen)"
          : note === "enlarged"
            ? "Window: Fit to video aspect ratio (enlarged to the minimum size)"
            : "Window: Fit to video aspect ratio",
      ),
    );
  }

  /** Mini Size on / off. */
  async toggleMini() {
    if (this.mini) {
      const { previous } = this.mini;
      this.#endMini();
      if (!this.#plugin.window.state.fullscreen) await this.#resize(previous, false);
      quiet(this.#plugin.osd.show("Mini size: Off"));
      return;
    }
    const video = await this.#needVideo();
    if (!video) return;
    const previous = { ...this.#plugin.window.state.videoArea };
    const applied = await this.#resize(mini(video), false);
    if (!applied) return;
    this.mini = { previous, applied: { width: applied.width, height: applied.height } };
    this.#plugin.update();
    quiet(this.#plugin.osd.show("Mini size: On"));
  }

  /**
   * G-02: `choice` for a source that just reported its video size. Silent
   * (no OSD, not even when the core shrinks it to the screen or refuses);
   * nothing is sent when the video area already has that size.
   */
  async auto(choice: Exclude<AutoSize, "off">, video: Size) {
    const area = this.#plugin.window.state.videoArea;
    const target = choice === "fit" ? fitted(area, video) : scaled(video, choice);
    if (near(area, target)) return;
    await this.#resize(target, false, true);
  }

  /** End Mini Size if it is on, back to the size before it (picture-in-picture is about to start). */
  async leaveMini() {
    if (this.mini) await this.toggleMini();
  }

  #endMini() {
    this.mini = null;
    this.#plugin.update();
  }

  async #needVideo() {
    const video = await this.#video.refresh();
    if (!video) quiet(this.#plugin.osd.show("No video"));
    return video;
  }

  /** `leaveMini`: an explicit size ends Mini Size (its previous size is dropped); `silent`: no OSD on failure. */
  async #resize(size: Size, leaveMini: boolean, silent = false) {
    if (leaveMini && this.mini) this.#endMini();
    this.resizing = true;
    this.#plugin.update();
    try {
      return await this.#plugin.window.setVideoArea({ width: Math.round(size.width), height: Math.round(size.height) });
    } catch (error) {
      if (!silent)
        quiet(this.#plugin.osd.show("Cannot resize the window: {}", [error instanceof Error ? error.message : String(error)]));
      return null;
    } finally {
      this.resizing = false;
      this.#plugin.update();
    }
  }
}
