import type { CommandState, TemplateArgs } from "@vestra-player/sdk";

/**
 * The English source texts this plugin hands the player, as literal types:
 * the commands and the on-screen messages go through helpers, so
 * `vestra-plugin i18n extract` reads them from these types. The player
 * translates them with `locales/<lang>.po`.
 */

/** Menu path segments (`commands.add` path). `@open`, `@playback` are the player's menus. */
export type MenuText =
  | "@open"
  | "@playback"
  // The player's own submenus of Playback (doc/architecture/i18n.md §5).
  | "Seek"
  | "Speed"
  | "Open File…"
  | "Open URL…"
  | "Reopen Current/Last File"
  | "Copy to Clipboard"
  | "File Name"
  | "Folder Path"
  | "Full Path"
  | "Previous Keyframe"
  | "Next Keyframe"
  | "Jump to Time…"
  | "Jump to Start"
  | "Seek Steps…"
  | "0.5×"
  | "1.5×"
  | "2×"
  | "Custom…"
  | "Disable Screen Saver While Playing"
  | "Show Time on Seek Bar Hover"
  | "Show Thumbnail on Seek Bar Hover";

/** Why a command is greyed out (`reason`). */
export type Reason =
  | "Not supported by the current source"
  | "No file to reopen"
  | "No open file"
  | "A network source has no file name"
  | "No video"
  | "Needs ffmpeg";

/** {@link CommandState} with a reason this plugin can translate. */
export type State = Omit<CommandState, "reason"> & { reason: Reason | null };

/** On-screen messages (`osd.show` templates; `{}` takes the args in order). */
export type OsdText =
  | "Seek: {} / {}"
  | "Back {} s: {} / {}"
  | "Forward {} s: {} / {}"
  | "Jumped to the start"
  | "Previous keyframe: {} / {}"
  | "Next keyframe: {} / {}"
  | "Already at the last keyframe"
  | "Speed: {}x"
  | "Volume: {}%"
  | "Open failed"
  | "Open failed: {}"
  | "Reopened: {}"
  | "Reopened the last file: {}"
  | "Copied: {}"
  | "Copy failed: {}"
  | "Only the first line is opened; to open several links, add them to the playlist"
  | "Disable screen saver while playing: On"
  | "Disable screen saver while playing: Off"
  | "Show time on seek bar hover: On"
  | "Show time on seek bar hover: Off"
  | "Show thumbnail on seek bar hover: On"
  | "Show thumbnail on seek bar hover: Off"
  | FailureText;

/** Shows one on-screen message; headless hosts refuse it, which never fails the action. */
export type Osd = (text: OsdText, args?: TemplateArgs) => void;

/**
 * A failure the plugin itself words. `kind` picks the sentence (the tool
 * windows show it from their Slint file); `detail` is the player's message
 * for `source`, shown as it is.
 */
export type FailureKind = "source" | "time-range" | "rate-range";
const FAILURES = {
  source: "Source operation failed: {}",
  "time-range": "Enter a time within the source's duration as HH:MM:SS.mmm",
  "rate-range": "Speed must be between 0.25× and 4×",
} as const;
type FailureText = (typeof FAILURES)[FailureKind];

export class Failure extends Error {
  readonly kind: FailureKind;
  readonly detail: string;
  constructor(kind: FailureKind, detail = "") {
    super(detail ? `${kind}: ${detail}` : kind);
    this.kind = kind;
    this.detail = detail;
  }
  /** Show it on the video. */
  show(osd: Osd) {
    if (this.kind === "source") osd(FAILURES.source, [this.detail]);
    else osd(FAILURES[this.kind]);
  }
}

export const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Wrap a failure of a source operation (the bar's former 「来源操作失败：」 prefix). */
export const sourceFailure = (error: unknown) => new Failure("source", errorMessage(error));
