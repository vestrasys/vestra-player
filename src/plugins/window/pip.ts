import type { Plugin, PluginError } from "@vestra-player/sdk";
import { quiet } from "./osd.ts";
import type { VideoSize } from "./video-size.ts";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * H07 Picture-in-Picture where the player supports it (macOS): the system
 * picture-in-picture window through `window.setPictureInPicture`. The check
 * mark follows `state.pictureInPicture.active`; when the window closes by
 * itself (its return / close buttons, the video went away) the change is said
 * once. Fullscreen is left by the core, so it does not disable the command.
 */
export class PictureInPicture {
  readonly #plugin: Plugin;
  readonly #video: VideoSize;
  /** A `setPictureInPicture` call is in flight; its own result is said by `toggle`. */
  switching = false;
  #active: boolean;

  constructor(plugin: Plugin, video: VideoSize) {
    this.#plugin = plugin;
    this.#video = video;
    this.#active = plugin.window.state.pictureInPicture.active;
    plugin.player.on("window.changed", (state) => {
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
      quiet(this.#plugin.osd.show("No video"));
      return;
    }
    this.switching = true;
    this.#plugin.update();
    try {
      await this.#plugin.window.setPictureInPicture(on);
      quiet(this.#plugin.osd.show(on ? "Picture-in-picture: On" : "Picture-in-picture: Off"));
    } catch (error) {
      if ((error as PluginError).code === "busy") return;
      quiet(
        this.#plugin.osd.show(on ? "Cannot enter picture-in-picture: {}" : "Cannot exit picture-in-picture: {}", [
          message(error),
        ]),
      );
    } finally {
      this.switching = false;
      this.#active = this.active;
      this.#plugin.update();
    }
  }

  /** Closed from the small window or by the core (no video, source closed). */
  async #closed() {
    const video = this.#video.showing ? await this.#video.refresh().catch(() => null) : null;
    quiet(this.#plugin.osd.show(video ? "Picture-in-picture: Off" : "Picture-in-picture: Off (no video)"));
  }
}
