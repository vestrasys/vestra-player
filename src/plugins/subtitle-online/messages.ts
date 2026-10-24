// The status lines of the windows and the on-screen messages: an id and its
// arguments (data). The English templates are the source text; the windows
// show them through ui/messages.slint (the same texts, @tr), the OSD through
// `osd.show(STATUS[id], args)`. tests/i18n.test.ts checks that both agree.
// formats.ts (shared with the Subtitles plugin) throws `Failure`s with its ids.

/** Every status sentence, by id. */
export const STATUS = {
  // Download window
  privacy: "Searching sends the file name keywords and the file hash to the chosen service.",
  noService:
    "First enter an API key for OpenSubtitles under Services (free at opensubtitles.com), or turn on Assrt and enter its token.",
  noMedia: "No media to load subtitles into",
  noMediaSearchTitle: "No media to load subtitles into; you can search by title.",
  sourceChanged: "The media changed; no subtitle was loaded.",
  cancelled: "Cancelled.",
  invalid: "The player refused the subtitle (file too large or invalid).",
  unsupported: "Subtitles cannot be loaded into the current media.",
  failed: "Loading the subtitle failed.",
  enterTitle: "Enter a title, then search.",
  searching: "Searching…",
  found: "Subtitles found: {}",
  notFound: "No subtitles found; change the title and search again",
  downloading: "Downloading subtitle…",
  downloadingOf: "Downloading subtitles ({}/{})…",
  opened: "Opened subtitle: {}",
  notSavedBeside: "Subtitle loaded, but it could not be saved in the video's folder",
  savedTo: "Saved to {}",
  added: "Subtitles added: {}",
  savedCount: "Subtitles saved: {} (in {})",
  nowhereToSave: "No place to save to.",
  notSaved: "Not saved.",
  resetAt: "Resets at {}",
  autoLoaded: "Online subtitle loaded automatically: {}",
  archiveUnsupported: "This archive format (rar, 7z) is not supported yet; download a zip or a single subtitle file.",
  archiveEmpty: "The archive has no subtitle files.",
  archiveBroken: "The archive is damaged",
  // Settings windows
  languagesHint: "Write preferred languages like zh-CN, zh-TW, en (at most 5)",
  testing: "Testing…",
  "testing.opensubtitles": "Testing OpenSubtitles…",
  "testing.assrt": "Testing Assrt…",
  saved: "Saved ({})",
  saveFailed: "Could not save: {}",
  keySet: "Key entered",
  tokenSet: "Token entered",
  keyMissing: "No key",
  tokenMissing: "No token",
  // A service's test
  keyFirst: "Enter the API key first",
  keyAccountValid: "Key and account are valid",
  keyAccountRemaining: "Key and account are valid; downloads left today: {}",
  keyValid: "Key is valid (not logged in: fewer anonymous downloads)",
  tokenFirst: "Enter the token first",
  tokenValid: "Token is valid",
  tokenRemaining: "Token is valid; calls left: {}",
  // A service's errors
  aborted: "Cancelled",
  "connect.opensubtitles": "Cannot connect to OpenSubtitles",
  "connect.assrt": "Cannot connect to Assrt",
  "unreadable.opensubtitles": "OpenSubtitles returned an unreadable response",
  "unreadable.assrt": "Assrt returned an unreadable response",
  osBadKey: "Invalid API key; check it in Online Subtitle Settings",
  osQuota: "No downloads left today (OpenSubtitles). Log in for more downloads",
  osTooMany: "Too many requests to OpenSubtitles; try again later",
  osError: "OpenSubtitles error ({})",
  osBadLogin: "Wrong OpenSubtitles user name or password",
  osLoginFailed: "OpenSubtitles login failed",
  osNoLink: "OpenSubtitles gave no download link",
  downloadFailed: "Cannot download the subtitle ({})",
  assrtBadToken: "Invalid Assrt token; check it in Online Subtitle Settings",
  assrtQuota: "Assrt call limit reached; try again later",
  assrtError: "Assrt error ({})",
  assrtNoLink: "Assrt gave no download link",
  // Reading a subtitle file (formats.ts, the same texts as in the Subtitles plugin).
  readFailed: "Cannot read the subtitle file.",
  notRegular: "The subtitle file must be a regular file of 1 byte to 8 MiB.",
  emptyOrLarge: "The subtitle file is empty or larger than 8 MiB.",
  bitmap: "Image subtitle files are not supported yet",
  unknownFormat: "Unknown subtitle format",
  noCues: "The subtitle file has no subtitles.",
  tooLarge: "The subtitle file is larger than 2 MiB",
  // Notes after a status
  noteEncoding: "The subtitle encoding may be wrong; change the default encoding in Subtitle Settings",
  noteFonts: "Fonts embedded in the subtitle were ignored",
  noteFps: "Unknown video frame rate; the MicroDVD subtitle was converted at 23.976",
} as const;

export type StatusId = keyof typeof STATUS;
export type NoteId = "noteEncoding" | "noteFonts" | "noteFps";

/**
 * One sentence: an id and its arguments (data, never translated). `raw` shows
 * a message of the player, the system or a service as it is (args[0]); `""` is none.
 */
export type Line = { id: StatusId | "raw" | ""; args: (string | number)[] };
export const line = (id: StatusId | "", args: (string | number)[] = []): Line => ({ id, args });
export const raw = (text: string): Line => ({ id: "raw", args: [text] });

/**
 * A window's status: up to five lines joined as sentences ("A. B") or as a
 * list ("A; B"), or "<service>: <line>" (kind "service").
 */
export type Status = { kind: "sentences" | "list" | "service"; service?: string; parts: Line[] };
export const NONE: Status = { kind: "sentences", parts: [] };
export const sentences = (...parts: Line[]): Status => ({ kind: "sentences", parts });
export const list = (...parts: Line[]): Status => ({ kind: "list", parts });
export const serviceResult = (service: string, result: Line): Status => ({ kind: "service", service, parts: [result] });

/** A failure the plugin itself reports: its line. */
export class Failure extends Error {
  readonly status: Line;
  constructor(id: StatusId, args: (string | number)[] = []) {
    super(STATUS[id]);
    this.status = line(id, args);
  }
}

/** What Slint gets (ui/messages.slint `Line` and `Status`). */
export const lineForSlint = (l: Line | undefined) => ({ id: l?.id ?? "", a: String(l?.args[0] ?? ""), b: String(l?.args[1] ?? "") });
export const forSlint = (s: Status) => ({
  kind: s.kind,
  service: s.service ?? "",
  p0: lineForSlint(s.parts[0]),
  p1: lineForSlint(s.parts[1]),
  p2: lineForSlint(s.parts[2]),
  p3: lineForSlint(s.parts[3]),
  p4: lineForSlint(s.parts[4]),
});
