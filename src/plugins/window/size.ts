import type { Plugin } from "../../sdk/plugin/index.ts";
import { clampNote, fitted, mini, near, scaled, type Size } from "./geometry.ts";
import type { VideoSize } from "./video-size.ts";

/** Why the size commands are disabled, `null` when they can run. */
export function sizeReason(plugin: Plugin, video: VideoSize) {
  if (plugin.window.state.fullscreen) return "全屏时不可用";
  if (!video.size) return "没有视频";
  return null;
}

/**
 * H03 倍数尺寸, H04 适应视频比例 and 迷你尺寸 (H07 where the system has
 * no picture-in-picture): every resize goes through `setVideoArea`, one at
 * a time (`busy` meanwhile).
 * 迷你尺寸 remembers the video area before it and ends by itself when the
 * window is resized or goes fullscreen.
 */
export class Sizer {
  readonly #plugin: Plugin;
  readonly #video: VideoSize;
  readonly #osd: (text: string) => void;
  readonly #changed: () => void;
  resizing = false;
  /** While 迷你尺寸 is on: the video area to go back to, and the one it set. */
  mini: { previous: Size; applied: Size } | null = null;

  constructor(plugin: Plugin, video: VideoSize, osd: (text: string) => void, changed: () => void) {
    this.#plugin = plugin;
    this.#video = video;
    this.#osd = osd;
    this.#changed = changed;
    plugin.window.onChange((state) => {
      if (!this.mini || this.resizing) return;
      if (state.fullscreen || !near(state.videoArea, this.mini.applied)) this.#endMini();
    });
  }

  /** H03: `scale` times the video's display size. */
  async scale(scale: number, title: string) {
    const video = await this.#needVideo();
    if (!video) return;
    const target = scaled(video, scale);
    const applied = await this.#resize(target, true);
    if (applied) this.#osd(`窗口: ${title}${clampNote(target, applied)}`);
  }

  /** H04: same width, the video's aspect ratio. */
  async fit() {
    const video = await this.#needVideo();
    if (!video) return;
    const area = this.#plugin.window.state.videoArea;
    const target = fitted(area, video);
    if (near(area, target)) {
      this.#osd("窗口已适应视频比例");
      return;
    }
    const applied = await this.#resize(target, true);
    if (applied) this.#osd(`窗口: 适应视频比例${clampNote(target, applied)}`);
  }

  /** 迷你尺寸 on / off. */
  async toggleMini() {
    if (this.mini) {
      const { previous } = this.mini;
      this.#endMini();
      if (!this.#plugin.window.state.fullscreen) await this.#resize(previous, false);
      this.#osd("迷你尺寸: 关");
      return;
    }
    const video = await this.#needVideo();
    if (!video) return;
    const previous = { ...this.#plugin.window.state.videoArea };
    const applied = await this.#resize(mini(video), false);
    if (!applied) return;
    this.mini = { previous, applied: { width: applied.width, height: applied.height } };
    this.#changed();
    this.#osd("迷你尺寸: 开");
  }

  /** End 迷你尺寸 if it is on, back to the size before it (the form switched to 画中画). */
  async leaveMini() {
    if (this.mini) await this.toggleMini();
  }

  #endMini() {
    this.mini = null;
    this.#changed();
  }

  async #needVideo() {
    const video = await this.#video.refresh();
    if (!video) this.#osd("没有视频");
    return video;
  }

  /** `leaveMini`: an explicit size ends 迷你尺寸 (its previous size is dropped). */
  async #resize(size: Size, leaveMini: boolean) {
    if (leaveMini && this.mini) this.#endMini();
    this.resizing = true;
    this.#plugin.update();
    try {
      return await this.#plugin.window.setVideoArea({ width: Math.round(size.width), height: Math.round(size.height) });
    } catch (error) {
      this.#osd(`无法调整窗口：${error instanceof Error ? error.message : String(error)}`);
      return null;
    } finally {
      this.resizing = false;
    }
  }
}
