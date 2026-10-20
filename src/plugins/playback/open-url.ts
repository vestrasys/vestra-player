import { fileURLToPath } from "node:url";
import type { Scope, Window } from "../../sdk/plugin/index.ts";
import type { SettingsStore } from "./settings.ts";
import { system } from "./system.ts";

const SCHEMES = new Set(["http", "https", "rtsp", "rtmp", "rtmps", "mms", "ftp"]);
export const INVALID = "不是有效的链接";
export const FIRST_LINE_ONLY = "只打开第一行；多个链接请用播放列表的「添加链接…」";

/**
 * What `player.open` receives for one line: a supported network address as
 * typed, a local absolute path (`file://` converted), or `null`.
 */
export function parseAddress(line: string): string | null {
  const text = line.trim();
  if (text.startsWith("/")) return text;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (/\s/.test(text)) return null;
  const scheme = url.protocol.slice(0, -1).toLowerCase();
  if (scheme === "file") {
    try {
      return fileURLToPath(url);
    } catch {
      return null;
    }
  }
  return SCHEMES.has(scheme) && url.host !== "" ? text : null;
}

const lines = (text: string) =>
  text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");

/**
 * A02 打开链接…: a tool window that replaces the current source with the
 * address typed (or taken from the clipboard). It does not belong to a source:
 * the user's open goes ahead even if something else changed the source.
 */
export class OpenUrl {
  readonly window: Window;
  busy = false;
  message = "";
  /** The text of the last confirm or cancel in this run (never saved). */
  lastInput = "";
  #selectAll = false;
  #showing: Promise<void> | null = null;
  readonly #scope: Scope;
  readonly #settings: SettingsStore;

  constructor(scope: Scope, settings: SettingsStore, osd: (text: string) => void) {
    this.#scope = scope;
    this.#settings = settings;
    this.window = scope.windows.create("ui/open-url.slint", { id: "playback-open-url" });
    this.window.bind(() => ({
      message: this.message,
      busy: this.busy,
      selectAll: this.#selectAll,
      pasteFromClipboard: this.#settings.value.openUrl.pasteFromClipboard,
    }));
    this.window.on("pasteChanged", (on: boolean) =>
      this.#settings.update((s) => {
        s.openUrl.pasteFromClipboard = on;
      }),
    );
    this.window.on("cancel", async (value: string) => {
      // A running open goes ahead; closing only stops waiting for it here.
      this.lastInput = value;
      await this.window.close();
    });
    this.window.on("confirm", async (value: string) => {
      if (this.busy) return;
      this.lastInput = value;
      const all = lines(value);
      const target = all.length ? parseAddress(all[0]!) : null;
      if (target === null) {
        this.message = INVALID;
        return;
      }
      const note = all.length > 1 ? FIRST_LINE_ONLY : "";
      this.message = note;
      this.busy = true;
      try {
        await this.#scope.player.open(target);
      } catch (error) {
        this.busy = false;
        this.message = `打开失败：${error instanceof Error ? error.message : String(error)}`;
        if (!this.window.opened) osd(this.message); // cancelled while opening
        return;
      }
      this.busy = false;
      this.message = "";
      await this.window.close();
      if (note) osd(note); // the window is gone, so the note goes on the video
    });
  }

  /** Ctrl+U: open the window, or bring the open one forward unchanged. */
  show() {
    if (this.window.opened) return this.window.show();
    this.#showing ??= this.#open().finally(() => (this.#showing = null));
    return this.#showing;
  }

  async #open() {
    let value = this.lastInput;
    let selectAll = false;
    if (this.#settings.value.openUrl.pasteFromClipboard) {
      const first = lines(await system.readClipboard().catch(() => ""))[0];
      if (first !== undefined && parseAddress(first) !== null) [value, selectAll] = [first, true];
    }
    this.message = "";
    this.#selectAll = selectAll;
    this.window.set({ value, selectAll });
    await this.window.show();
  }
}
