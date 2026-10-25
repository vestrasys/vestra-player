// The status line of the windows and its on-screen message: one id and its
// arguments. The English templates are the source text; the windows and the
// OSD get the same text value (`statusText`), which the player translates.

import type { I18n, TextArg, TextValue } from "@vestra-player/sdk";

/** Every status sentence, by id. `.waiting` / `.uncertain`: the player has not confirmed it yet. */
export const STATUS = {
  cancelled: "The operation was cancelled or the media changed.",
  stale: "The subtitle tracks changed. Adjust again.",
  invalid: "The player refused the subtitle (file too large or invalid).",
  playerBusy: "The player is busy. Try again later.",
  unsupported: "The current media does not support this.",
  backend: "The player cannot load this subtitle.",
  failed: "The subtitle operation failed.",
  cancelledByUser: "Cancelled.",
  refreshed: "Subtitle tracks refreshed.",
  select: "Subtitles: {}",
  "select.waiting": "Subtitles: {} (waiting to take effect)",
  "select.uncertain": "Subtitles: {} (could not confirm it took effect)",
  moved: "Subtitles: {}. Secondary subtitle turned off (it is now the primary)",
  "moved.waiting": "Subtitles: {} (waiting to take effect). Secondary subtitle turned off (it is now the primary)",
  "moved.uncertain": "Subtitles: {} (could not confirm it took effect). Secondary subtitle turned off (it is now the primary)",
  hidden: "Subtitles hidden",
  tracksChanged: "The subtitle tracks changed. Choose again.",
  secondTextOnly: "Cannot use it as the secondary subtitle: only text subtitles can be secondary",
  secondIsPrimary: "Cannot use it as the secondary subtitle: it is already the primary subtitle",
  secondary: "Secondary subtitle: {}",
  "secondary.waiting": "Secondary subtitle: {} (waiting to take effect)",
  "secondary.uncertain": "Secondary subtitle: {} (could not confirm it took effect)",
  secondaryOff: "Secondary subtitle: off",
  noFile: "No subtitle file chosen.",
  opened: "Opened subtitle: {}",
  "opened.waiting": "Opened subtitle: {} (waiting to take effect)",
  "opened.uncertain": "Opened subtitle: {} (could not confirm it took effect)",
  openedTracks: "Opened subtitle: {} ({} tracks)",
  "openedTracks.waiting": "Opened subtitle: {} ({} tracks) (waiting to take effect)",
  "openedTracks.uncertain": "Opened subtitle: {} ({} tracks) (could not confirm it took effect)",
  secondaryTracks: "Secondary subtitle: {} ({} tracks)",
  "secondaryTracks.waiting": "Secondary subtitle: {} ({} tracks) (waiting to take effect)",
  "secondaryTracks.uncertain": "Secondary subtitle: {} ({} tracks) (could not confirm it took effect)",
  notSecondary: "Subtitle loaded, but it cannot be the secondary subtitle",
  "notSecondary.waiting": "Subtitle loaded, but it cannot be the secondary subtitle (waiting to take effect)",
  "notSecondary.uncertain": "Subtitle loaded, but it cannot be the secondary subtitle (could not confirm it took effect)",
  noneToShow: "No subtitles to show.",
  shown: "Subtitles shown",
  cycle: "Subtitles: {} ({}/{})",
  "cycle.waiting": "Subtitles: {} ({}/{}) (waiting to take effect)",
  "cycle.uncertain": "Subtitles: {} ({}/{}) (could not confirm it took effect)",
  cycleOff: "Subtitles: off",
  noSelection: "No subtitle selected.",
  embeddedNoReload: "Embedded subtitles need no reload",
  notOurs: "This subtitle was not loaded by the Subtitles plugin and cannot be reloaded",
  reloadNotFound: "Cannot reload: subtitle file {} not found",
  reloadChanged: "Cannot reload: the content of subtitle file {} changed",
  reloadFailed: "Cannot reload: {}",
  "reload.readFailed": "Cannot reload: cannot read the subtitle file.",
  "reload.notRegular": "Cannot reload: the subtitle file must be a regular file of 1 byte to 8 MiB.",
  "reload.emptyOrLarge": "Cannot reload: the subtitle file is empty or larger than 8 MiB.",
  "reload.bitmap": "Cannot reload: image subtitle files are not supported yet",
  "reload.unknownFormat": "Cannot reload: unknown subtitle format",
  "reload.noCues": "Cannot reload: the subtitle file has no subtitles.",
  "reload.tooLarge": "Cannot reload: the subtitle file is larger than 2 MiB",
  unchanged: "Subtitle unchanged",
  reloaded: "Reloaded subtitle {}",
  autoReloaded: "Subtitle reloaded automatically",
  selectFirst: "Select a subtitle track first.",
  notStylable: "The current subtitle track cannot be adjusted.",
  noFont: "No subtitle font available.",
  styleUpdated: "Subtitle style updated",
  "styleUpdated.waiting": "Subtitle style updated (waiting to take effect)",
  "styleUpdated.uncertain": "Subtitle style updated (could not confirm it took effect)",
  styleReset: "Default style restored.",
  selectionChanged: "The selected subtitle changed. Choose again.",
  removed: "External subtitle removed.",
  mediaSwitched: "The media changed; the running operation was cancelled.",
  syncInteger: "Enter whole milliseconds, for example 1250 or -500.",
  syncRange: "The sync value must be between −{} and {} ms",
  switching: "Switching subtitle track…",
  removing: "Removing subtitle…",
  switchingSecondary: "Switching secondary subtitle…",
  chooseFile: "Choose a subtitle file…",
  chooseSecondaryFile: "Choose a secondary subtitle file…",
  togglingShown: "Switching subtitle display…",
  hiding: "Hiding subtitles…",
  closingSecondary: "Turning off the secondary subtitle…",
  reloading: "Reloading subtitle…",
  refreshing: "Refreshing…",
  applyingStyle: "Applying subtitle style…",
  resetting: "Restoring the default style…",
  // Reading a subtitle file (formats.ts).
  readFailed: "Cannot read the subtitle file.",
  notRegular: "The subtitle file must be a regular file of 1 byte to 8 MiB.",
  emptyOrLarge: "The subtitle file is empty or larger than 8 MiB.",
  bitmap: "Image subtitle files are not supported yet",
  unknownFormat: "Unknown subtitle format",
  noCues: "The subtitle file has no subtitles.",
  tooLarge: "The subtitle file is larger than 2 MiB",
  // Notes after a status, as further sentences ("A. B", in Chinese "A。B").
  noteEncoding: "The subtitle encoding may be wrong; change the default encoding in Subtitle Settings",
  noteFonts: "Fonts embedded in the subtitle were ignored",
  noteFps: "Unknown video frame rate; the MicroDVD subtitle was converted at 23.976",
} as const;

export type StatusId = keyof typeof STATUS;
/** Errors of reading a subtitle file. */
export type FileError = "readFailed" | "notRegular" | "emptyOrLarge" | "bitmap" | "unknownFormat" | "noCues" | "tooLarge";
export type NoteId = "noteEncoding" | "noteFonts" | "noteFps";
/** Ids with the three confirmation forms. */
export type Confirmable =
  | "select" | "moved" | "secondary" | "opened" | "openedTracks" | "secondaryTracks" | "notSecondary" | "cycle" | "styleUpdated";

/**
 * A status: an id and its arguments (data, or a text value such as a track's
 * fallback name). `raw` shows a message of the player or the system as it is
 * (args[0]); `""` is none.
 */
export type Status = { id: StatusId | "raw" | ""; args: TextArg[]; notes: NoteId[] };
export const status = (id: StatusId | "", args: TextArg[] = [], notes: NoteId[] = []): Status => ({ id, args, notes });
export const raw = (text: string): Status => ({ id: "raw", args: [text], notes: [] });
export const NONE = status("");

/** `id`, or its waiting / uncertain form. */
export function confirmed(
  id: Confirmable,
  state: "waiting" | "applied" | "uncertain" | undefined,
): StatusId {
  if (state === "waiting") return `${id}.waiting`;
  if (state === "uncertain") return `${id}.uncertain`;
  return id;
}

/** A failure the plugin itself reports: its status. */
export class Failure extends Error {
  readonly status: Status;
  constructor(id: StatusId, args: TextArg[] = []) {
    super(STATUS[id]);
    this.status = status(id, args);
  }
}

/**
 * What the windows' status line and the OSD show: the status and its notes
 * as sentences, a text value the player translates; the player's own words
 * as they are; `""` for none.
 */
export function statusText({ text, list }: Pick<I18n, "text" | "list">, s: Status): TextValue {
  if (s.id === "raw") return String(s.args[0] ?? "");
  if (s.id === "") return "";
  const head = text(STATUS[s.id], s.args);
  return s.notes.length ? list([head, ...s.notes.map((note) => text(STATUS[note]))], "sentences") : head;
}
