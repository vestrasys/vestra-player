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
  | "Show Thumbnail on Seek Bar Hover"
  | StepTitle;

/**
 * Titles of the tier 2–4 seeks with their default seconds (B06). Seconds the
 * user set in Seek Steps… have no title of their own: a menu title cannot take
 * arguments yet, so those show in English (round doc, 第 2 轮请求).
 */
export const STEP_TITLES = {
  backward: { 30: "Back 30 s", 60: "Back 60 s", 300: "Back 300 s" },
  forward: { 30: "Forward 30 s", 60: "Forward 60 s", 300: "Forward 300 s" },
} as const;
type StepTitle = (typeof STEP_TITLES)[keyof typeof STEP_TITLES][keyof (typeof STEP_TITLES)["backward"]];

/** A title built from data that has no translation (see {@link STEP_TITLES}). */
export type Untranslated = string & { readonly untranslated: true };

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
