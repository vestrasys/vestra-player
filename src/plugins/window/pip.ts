import type { Plugin, PluginError } from "../../sdk/plugin/index.ts";
import type { VideoSize } from "./video-size.ts";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * H07 画中画 where the player supports it (macOS): the system
 * picture-in-picture window through `window.setPictureInPicture`. The check
 * mark follows `state.pictureInPicture.active`; when the window closes by
 * itself (its 返回 / close buttons, the video went away) the change is said
 * once. Fullscreen is left by the core, so it does not disable the command.
 */
export class PictureInPicture {
  readonly #plugin: Plugin;
  readonly #video: VideoSize;
  readonly #osd: (text: string) => void;
  /** A `setPictureInPicture` call is in flight; its own result is said by `toggle`. */
  switching = false;
  #active: boolean;

  constructor(plugin: Plugin, video: VideoSize, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#video = video;
    this.#osd = osd;
    this.#active = plugin.window.state.pictureInPicture.active;
    plugin.window.onChange((state) => {
      const active = state.pictureInPicture.active;
      if (this.switching || active === this.#active) return;
      this.#active = active;
      if (!active) void this.#closed();
    });
  }

  get active() {
    return this.#plugin.window.state.pictureInPicture.active;
  }

  /** On / off; a press while the previous call is still switching is ignored (`busy`). */
  async toggle() {
    if (this.switching) return;
    const on = !this.active;
    if (on && !(await this.#video.refresh().catch(() => this.#video.size))) {
      this.#osd("没有视频");
      return;
    }
    this.switching = true;
    this.#plugin.update();
    try {
      await this.#plugin.window.setPictureInPicture(on);
      this.#osd(`画中画: ${on ? "开" : "关"}`);
    } catch (error) {
      if ((error as PluginError).code === "busy") return;
      this.#osd(`${on ? "无法进入画中画" : "无法退出画中画"}：${message(error)}`);
    } finally {
      this.switching = false;
      this.#active = this.active;
      this.#plugin.update();
    }
  }

  /** Closed from the small window or by the core (no video, source closed). */
  async #closed() {
    const { phase } = this.#plugin.player.state;
    const video = phase === "ready" || phase === "ended" ? await this.#video.refresh().catch(() => null) : null;
    this.#osd(video ? "画中画: 关" : "画中画: 关（没有视频）");
  }
}
