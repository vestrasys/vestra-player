import { lstat } from "node:fs/promises";
import { basename, dirname, extname, join, resolve } from "node:path";
import type { PlayerState } from "@vestra-player/sdk";

/** Pure capture rules: parsing, validation, ranges and file naming. */
export type Kind = "image" | "burst" | "clip";
export interface Spec {
  kind: Kind;
  start: number;
  end?: number;
  count: number;
  step: number;
  processed: boolean;
  subtitles: boolean;
  /** Clip audio: a track id, `selected` or `none`. */
  track: string;
  /** Fix the current time when the capture is accepted. */
  current: boolean;
  /** Image file format for frames and bursts (F07); PNG when absent. Clips are always MP4. */
  format?: Format;
  /** JPEG quality 1–100. */
  quality?: number;
}

/** F07: the image formats a frame can be saved as. */
export type Format = "png" | "jpeg" | "bmp";
export const FORMATS: readonly Format[] = ["png", "jpeg", "bmp"];
export const FORMAT_LABEL: Record<Format, string> = { png: "PNG", jpeg: "JPEG", bmp: "BMP" };
export const extensionOf = (format: Format) => (format === "jpeg" ? ".jpg" : `.${format}`);
/** The format a user-typed file name asks for, or null for any other extension. */
export function formatOfName(path: string): Format | null {
  const extension = extname(path).toLowerCase();
  return extension === ".png" ? "png" : extension === ".jpg" || extension === ".jpeg" ? "jpeg" : extension === ".bmp" ? "bmp" : null;
}
/** JPEG quality: an integer 1–100, 90 when unusable. */
export function quality(value: unknown) {
  const n = Math.round(Number(value));
  return Number.isFinite(n) ? Math.min(100, Math.max(1, n)) : 90;
}

/**
 * Every reason the plugin gives in its own words (English source text). The
 * windows say the same texts through `Words.reason` in ui/parts.slint (a test
 * keeps the two lists equal); the OSD shows them through the tables below.
 */
export const REASON = {
  range: "Enter valid seconds; the end must be after the start.",
  burstLimits: "A burst needs 2–20 frames, an interval of 0.1–10 s and a known media duration.",
  burstPastEnd: "The burst runs past the end of the media. Use fewer frames or a shorter interval.",
  noName: "No free file name found.",
  unusable: "The current frame cannot be captured. Open a local video file first.",
  unexportable: "The current frame cannot be exported. Open a local video file first.",
  needDuration: "Bursts and clips need a known media duration.",
  outOfRange: "The selected range is past the end of the media.",
  track: "The selected audio track cannot be exported. Choose another track or No Audio.",
  changed: "The media changed; nothing was started.",
  busy: "The player is busy. Try again later.",
  unavailable: "The current frame cannot be captured.",
  captureFailed: "Could not capture the frame.",
  queueFull: "One export is running and four are queued. Please wait.",
  expired: "The kept frame has expired. Retry with the current frame.",
  released: "The kept frame was released.",
  cancelled: "Cancelled.",
  clipboard: "Could not write to the clipboard",
  space: "Not enough disk space",
  exists: "The target file already exists",
  permission: "No write permission",
  encoder: "This media cannot be encoded",
  exportFailed: "Export failed",
  noOutput: "This task has no output file yet.",
  outputGone: "The output file no longer exists.",
  cannotOpen: "Could not open the folder.",
  folderUnavailable: "The snapshot folder is unavailable. Please choose it again.",
} as const;
export type Reason = keyof typeof REASON;

/** The reasons an export job ends with (`jobError`, a released input). */
export type JobReason = "space" | "exists" | "permission" | "encoder" | "exportFailed" | "released";
const JOB_REASONS: readonly Reason[] = ["space", "exists", "permission", "encoder", "exportFailed", "released"];

/** A failure in the plugin's own words: `code` names its text in {@link REASON}. */
export class Failure extends Error {
  readonly code: Reason;
  constructor(code: Reason) {
    super(REASON[code]);
    this.code = code;
  }
}

/** A failure as data for a window or the OSD: the plugin's reason, or (reason "") the player's or system's own message. */
export type Problem<R extends string = Reason> = { reason: R | ""; raw: string };
export const problemOf = (error: unknown): Problem =>
  error instanceof Failure ? { reason: error.code, raw: "" } : { reason: "", raw: message(error) };
/** A task failure: a job reason, or the message of anything else. */
export const jobProblem = (error: unknown): Problem<JobReason> =>
  error instanceof Failure && JOB_REASONS.includes(error.code) ? { reason: error.code as JobReason, raw: "" } : { reason: "", raw: message(error) };

/** "Snapshot failed: …" — one template per reason a single frame (save, save as, copy, its task) can fail with. */
export const SNAPSHOT_FAILED = {
  folderUnavailable: "Snapshot failed: the snapshot folder is unavailable. Please choose it again.",
  queueFull: "Snapshot failed: one export is running and four are queued. Please wait.",
  unusable: "Snapshot failed: the current frame cannot be captured. Open a local video file first.",
  outOfRange: "Snapshot failed: the selected range is past the end of the media.",
  changed: "Snapshot failed: the media changed; nothing was started.",
  busy: "Snapshot failed: the player is busy. Try again later.",
  unavailable: "Snapshot failed: the current frame cannot be captured.",
  captureFailed: "Snapshot failed: could not capture the frame.",
  cancelled: "Snapshot failed: cancelled.",
  noName: "Snapshot failed: no free file name found.",
  space: "Snapshot failed: not enough disk space",
  exists: "Snapshot failed: the target file already exists",
  permission: "Snapshot failed: no write permission",
  encoder: "Snapshot failed: this media cannot be encoded",
  exportFailed: "Snapshot failed: export failed",
  released: "Snapshot failed: the kept frame was released.",
} as const satisfies Partial<Record<Reason, string>>;
/** "Burst failed: …" per job reason. */
export const BURST_FAILED = {
  space: "Burst failed: not enough disk space",
  exists: "Burst failed: the target file already exists",
  permission: "Burst failed: no write permission",
  encoder: "Burst failed: this media cannot be encoded",
  exportFailed: "Burst failed: export failed",
  released: "Burst failed: the kept frame was released.",
} as const satisfies Record<JobReason, string>;
/** "Export failed: …" (a clip) per job reason. */
export const CLIP_FAILED = {
  space: "Export failed: not enough disk space",
  exists: "Export failed: the target file already exists",
  permission: "Export failed: no write permission",
  encoder: "Export failed: this media cannot be encoded",
  exportFailed: "Export failed: export failed",
  released: "Export failed: the kept frame was released.",
} as const satisfies Record<JobReason, string>;

export const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function parseRange(start: unknown, end?: unknown) {
  const a = Number(String(start).trim());
  const b = end === undefined ? undefined : Number(String(end).trim());
  if (String(start).trim() === "" || !Number.isFinite(a) || a < 0 || (b !== undefined && (!Number.isFinite(b) || b <= a)))
    throw new Failure("range");
  return { start: a, end: b };
}

export function sampleTimes(start: number, count: number, step: number, duration: number) {
  if (!Number.isInteger(count) || count < 2 || count > 20 || !Number.isFinite(step) || step < 0.1 || step > 10 || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(start) || start < 0)
    throw new Failure("burstLimits");
  const result = Array.from({ length: count }, (_, index) => start + index * step);
  if (result[result.length - 1]! >= duration) throw new Failure("burstPastEnd");
  return result;
}

/** The first unused `name (n).ext` next to `path`; existing files and `taken` paths are never chosen. */
export async function uniqueFile(path: string, taken: ReadonlySet<string> = new Set()) {
  const absolute = resolve(path);
  const extension = extname(absolute);
  const stem = basename(absolute, extension);
  for (let i = 0; i < 1000; i++) {
    const candidate = i ? join(dirname(absolute), `${stem} (${i})${extension}`) : absolute;
    if (taken.has(candidate)) continue;
    try {
      await lstat(candidate);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return candidate;
      throw error;
    }
  }
  throw new Failure("noName");
}

/** `1:02:03.450` style time for file names and rows (no hours below one hour). */
export function clock(seconds: number, separator = ":") {
  const ms = Math.max(0, Math.round(seconds * 1000));
  const h = Math.floor(ms / 3_600_000), m = Math.floor(ms / 60_000) % 60, s = Math.floor(ms / 1000) % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return `${h ? `${h}${separator}` : ""}${two(m)}${separator}${two(s)}.${String(ms % 1000).padStart(3, "0")}`;
}

/** `movie_01.02.450.png`: source name plus media time. */
export function frameName(source: string | null, seconds: number, extension: string) {
  const stem = source ? basename(source.split(/[?#]/)[0]!, extname(source.split(/[?#]/)[0]!)) || "capture" : "capture";
  return `${stem.replace(/[\\/:*?"<>|]/g, "_")}_${clock(seconds, ".")}${extension}`;
}

export const usable = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.can.snapshot;

export function validate(spec: Spec, state: PlayerState) {
  if (!usable(state)) throw new Failure("unusable");
  const { duration } = state;
  if (spec.kind !== "image" && (duration === null || !Number.isFinite(duration))) throw new Failure("needDuration");
  const start = spec.current ? (state.position ?? 0) : spec.start;
  if (spec.kind === "burst") sampleTimes(start, spec.count, spec.step, duration!);
  if (duration !== null && (start >= duration || (spec.end !== undefined && spec.end > duration))) throw new Failure("outOfRange");
}

/** One source-internal audio track for a clip, or none. */
export function chooseAudio(spec: Spec, state: PlayerState): number | "none" {
  if (spec.kind !== "clip" || spec.track === "none") return "none";
  const tracks = state.audioTracks;
  const track = spec.track === "selected" ? tracks?.find((t) => t.selected) : tracks?.find((t) => t.id === Number(spec.track));
  if (!track && spec.track === "selected" && !tracks?.length) return "none";
  if (!track || !track.exportable) throw new Failure("track");
  return track.id;
}

/**
 * Clip audio choices: index 0 none, 1 the selected track, then each track.
 * The labels stay composed here, in Chinese, until a window can build a
 * combo box list from translated words plus data (migration blocker, see
 * doc/process/rounds/2026-10-i18n.md「第 2 轮请求」).
 */
export function audioChoices(state: PlayerState) {
  const tracks = state.audioTracks ?? [];
  return {
    labels: /* i18n-ignore: migration blocker (above) */ ["无声音", "当前音轨", ...tracks.map((t) => `音轨 ${t.id}${t.title || t.language ? ` · ${t.title ?? t.language}` : ""}`)],
    values: ["none", "selected", ...tracks.map((t) => String(t.id))],
  };
}
