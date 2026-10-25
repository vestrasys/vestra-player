import { fileURLToPath } from "node:url";
import type { Scope, Window } from "@vestra-player/sdk";
import type { SettingsStore } from "./settings.ts";
import { errorMessage, type Osd } from "./texts.ts";

const SCHEMES = new Set(["http", "https", "rtsp", "rtmp", "rtmps", "mms", "ftp"]);
export const FIRST_LINE_ONLY = "Only the first line is opened; to open several links, add them to the playlist";

/**
 * The window's message line; `ui/open-url.slint` has the texts: `invalid`
 * (not a link), `firstLine` ({@link FIRST_LINE_ONLY}), `failed` (the
 * player's message in `detail`); "" for none.
 */
export type UrlNotice = "" | "invalid" | "firstLine" | "failed";

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
 * A02 Open URL…: a tool window that replaces the current source with the
 * address typed (or taken from the clipboard). It does not belong to a source:
 * the user's open goes ahead even if something else changed the source.
 */
export class OpenUrl {
  readonly window: Window;
  busy = false;
  notice: UrlNotice = "";
  detail = "";
  /** The text of the last confirm or cancel in this run (never saved). */
  lastInput = "";
  #selectAll = false;
  #showing: Promise<void> | null = null;
  readonly #scope: Scope;
  readonly #settings: SettingsStore;

  constructor(scope: Scope, settings: SettingsStore, osd: Osd) {
    this.#scope = scope;
    this.#settings = settings;
    this.window = scope.windows.create("ui/open-url.slint", { id: "playback-open-url" });
    this.window.bind(() => ({
      notice: this.notice,
      detail: this.detail,
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
        this.#tell("invalid");
        return;
      }
      const note = all.length > 1;
      this.#tell(note ? "firstLine" : "");
      this.busy = true;
      try {
        await this.#scope.player.open(target);
      } catch (error) {
        this.busy = false;
        this.#tell("failed", errorMessage(error));
        if (!this.window.opened) osd("Open failed: {}", [this.detail]); // cancelled while opening
        return;
      }
      this.busy = false;
      this.#tell("");
      await this.window.close();
      if (note) osd(FIRST_LINE_ONLY); // the window is gone, so the note goes on the video
    });
  }

  #tell(notice: UrlNotice, detail = "") {
    this.notice = notice;
    this.detail = detail;
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
      // No text, or no access: nothing to fill in, and nothing to report.
      const text = await this.#scope.clipboard.readText().catch(() => null);
      const first = lines(text ?? "")[0];
      if (first !== undefined && parseAddress(first) !== null) [value, selectAll] = [first, true];
    }
    this.#tell("");
    this.#selectAll = selectAll;
    this.window.set({ value, selectAll });
    await this.window.show();
  }
}
