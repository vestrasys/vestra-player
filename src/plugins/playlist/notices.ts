import type { I18n, TemplateArgs, Text, TextList } from "@vestra-player/sdk";

/**
 * Every notice the playlist shows on screen (`osd.show`): English templates
 * the player translates with `locales/<lang>.po`; `{}` and `{name}` take the
 * arguments (data: counts, file names, messages of the system, or texts).
 * Notices made of a varying number of parts are text values instead
 * (`Queue.summary`, `problemClauses`, Show in Finder).
 */
export const NOTICES = [
  // Adding (the summary is a list of clauses, see `Queue.summary`).
  "Added {count} files from the same folder",
  "Added {count} files from the same folder, limit of {limit} reached",
  "Nothing to add",
  "Link added",
  "Existing link selected",
  // Folders.
  "Scanning folder…",
  "No playable files in the folder",
  "Cannot read the folder: {}",
  // Removing.
  "Removed: {}",
  "Invalid items removed: {}",
  "No invalid items",
  "The media being opened was removed from the list",
  "The current media was removed from the list",
  "List cleared; the current media continues",
  // List files.
  "Unsupported list format",
  "Cannot open the list: the file is larger than 16 MB",
  "Cannot open the list: {}",
  "Opened list: {name} ({count} items)",
  "Opened list: {name} ({count} items), skipped: {skipped}",
  "Cannot save the list: {}",
  "Saved list: {}",
  // Options.
  "Repeat: Off",
  "Repeat: All",
  "Repeat: One",
  "Shuffle: On",
  "Shuffle: Off",
  "Fast advance to next: On",
  "Fast advance to next: Off",
  "The list will be remembered",
  "The list will no longer be remembered",
  "Confirm before removing all: On",
  "Confirm before removing all: Off",
  // Playing.
  "End of the list",
  "Cannot replay: {}",
  "Cannot open {}: the file does not exist",
  "Cannot open {}: cannot open this media",
  "Cannot open {}: {}",
  "The file's canonical path matches an existing item; that item is selected",
  "Close the other dialog first",
] as const;

export type Notice = (typeof NOTICES)[number];
/** Positional (`{}`) or named (`{name}`) arguments of a notice. */
export type NoticeArgs = TemplateArgs;
/** What `Queue.notice` shows: a template of `NOTICES` (with `NoticeArgs`), or a text value. */
export type NoticeText = Notice | Text | TextList;
/** The text values of `plugin.i18n` (passed in, so the list logic stays testable). */
export type Texts = Pick<I18n, "text" | "list">;

/**
 * Why a folder or list file could not be read: one of the fixed reasons
 * (each shown as a whole sentence), else the system's message.
 */
export type ReadFailure =
  | { reason: "permission" | "access" | "missing" | "not-folder" | "large" }
  | { reason: "message"; message: string };

/** At most `max` characters of a name shown in a notice or confirmation (both are 1–200 characters). */
export const fitName = (name: string, max = 120) =>
  [...name].length > max ? `${[...name].slice(0, max - 1).join("")}…` : name;

/** Why a folder or list file could not be read, as words (a fixed reason), else the system's message. */
export function failureText({ text }: Texts, failure: ReadFailure, max = 120): Text | string {
  switch (failure.reason) {
    case "permission":
      return text("access to this location is not allowed (System Settings ▸ Privacy & Security ▸ Files & Folders)");
    case "access":
      return text("no permission to read");
    case "missing":
      return text("the folder does not exist");
    case "not-folder":
      return text("not a folder");
    case "large":
      return text("the file is larger than 16 MB");
    case "message":
      return fitName(failure.message, max);
  }
}
