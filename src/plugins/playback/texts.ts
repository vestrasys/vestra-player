import type { CommandState, TemplateArgs } from "@vestra-player/sdk";

/**
 * The English source texts this plugin hands the player, as literal types:
 * the commands and the on-screen messages go through helpers, so
 * `vestra-plugin i18n extract` reads them from these types. The player
 * translates them with `locales/<lang>.po`.
 */

/** The player's menus a path starts in. */
export type Anchor = "@open" | "@playback";

/** Menu path segments and the titles of commands outside the menu (`commands.add` `path` and `title`). */
export type MenuText =
  | "Playback…"
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

/**
 * What a command is about (`category`, player 0.7.6): the command palette
 * shows "Category: Title". A submenu's commands take its name; a top-level
 * item takes its area.
 */
export type Category = "File" | "Copy to Clipboard" | "Seek" | "Speed" | "Playback";

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
  | LoopOsdText
  | FailureText;

/** B23 A-B Loop (moved here from the marks plugin with the same texts). */
export type LoopOsdText =
  | "A-B loop: {} ~ {}"
  | "A-B start: {}"
  | "A-B end: {}"
  | "A-B loop ended"
  | "A-B loop off"
  | "Start point is after the end point; end point cleared"
  | "End point must be after the start point"
  | "Start point cleared"
  | "End point cleared"
  | "Reached the beginning"
  | "Reached the end"
  | "Set the start and end points first"
  | "No chapters or bookmarks to define a range"
  | "Range too short to loop"
  | "Loop count: endless"
  | "Loop count: {}"
  | "This source doesn't support A-B loop"
  | "A-B loop failed: {}";

/** Shows one on-screen message; headless hosts refuse it, which never fails the action. */
export type Osd = (text: OsdText, args?: TemplateArgs) => void;

/**
 * A failure the plugin itself words. `kind` picks the sentence, shown on the
 * video (the Playback panel marks the refused field); `detail` is the player's message
 * for `source`, shown as it is.
 */
export type FailureKind = "source" | "time-range" | "rate-range" | "step-range";
const FAILURES = {
  source: "Source operation failed: {}",
  "time-range": "Enter a time within the source's duration as HH:MM:SS.mmm",
  "rate-range": "Speed must be between 0.25× and 4×",
  "step-range": "Range 0.1–3600 s, at most one decimal place",
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
