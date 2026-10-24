/**
 * Every notice the playlist shows on screen (`osd.show`): English templates
 * the player translates with `locales/<lang>.po`; `{}` and `{name}` take the
 * arguments (data: counts, file names, messages of the system).
 */
export const NOTICES = [
  // Adding (the summary: what was added, then the first problem).
  "Added: {added}",
  "Added: {added}, duplicates skipped: {duplicates}",
  "Added: {added}, not readable for now: {count}",
  "Added: {added}, duplicates skipped: {duplicates}, not readable for now: {count}",
  "Added: {added}, unsupported items skipped: {count}",
  "Added: {added}, duplicates skipped: {duplicates}, unsupported items skipped: {count}",
  "Added: {added}, subfolders that could not be read: {count}",
  "Added: {added}, duplicates skipped: {duplicates}, subfolders that could not be read: {count}",
  "Added: {added}, limit of {limit} reached",
  "Added: {added}, duplicates skipped: {duplicates}, limit of {limit} reached",
  "Added: {added}, could not read {name}",
  "Added: {added}, duplicates skipped: {duplicates}, could not read {name}",
  "Added: {added}, the list is full at {limit} items",
  "Added: {added}, duplicates skipped: {duplicates}, the list is full at {limit} items",
  "Added: {added}; another source is opening, only added to the list",
  "Added: {added}; opening, added to the list",
  "Added {count} files from the same folder",
  "Added {count} files from the same folder, limit of {limit} reached",
  "Nothing to add",
  "Nothing to add: unsupported items skipped: {count}",
  "Nothing to add: subfolders that could not be read: {count}",
  "Nothing to add: cannot read {name}: access to this location is not allowed (System Settings ▸ Privacy & Security ▸ Files & Folders)",
  "Nothing to add: cannot read {name}: no permission to read",
  "Nothing to add: cannot read {name}: the folder does not exist",
  "Nothing to add: cannot read {name}: not a folder",
  "Nothing to add: cannot read {name}: the file is larger than 16 MB",
  "Nothing to add: cannot read {name}: {message}",
  "Link added",
  "Existing link selected",
  // Folders.
  "Scanning folder…",
  "No playable files in the folder",
  "Cannot read the folder: access to this location is not allowed (System Settings ▸ Privacy & Security ▸ Files & Folders)",
  "Cannot read the folder: no permission to read",
  "Cannot read the folder: the folder does not exist",
  "Cannot read the folder: not a folder",
  "Cannot read the folder: {}",
  // Removing.
  "Removed: {}",
  "Invalid items removed: {}",
  "No invalid items",
  "The media being opened was removed from the list",
  "The current media was removed from the list",
  "List cleared; the current media continues",
  // Show in Finder.
  "Only the first {} files are shown",
  "File not found: {}",
  "Only the first {limit} files are shown; file not found: {name}",
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
export type NoticeArgs = (string | number)[] | Record<string, string | number>;

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
