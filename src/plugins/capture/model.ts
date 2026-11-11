import { lstat, mkdir } from "node:fs/promises";
import { basename, dirname, extname, isAbsolute, join, resolve } from "node:path";
import type { PlayerState } from "@vestra-player/sdk";

/** Pure capture rules: validation, sample times and file naming. */
export type Kind = "image" | "burst";
export interface Spec {
  kind: Kind;
  start: number;
  count: number;
  step: number;
  processed: boolean;
  subtitles: boolean;
  /** Fix the current time when the capture is accepted. */
  current: boolean;
  /** Image file format for frames and bursts (F07); PNG when absent. */
  format?: Format;
  /** JPEG quality 1–100. */
  quality?: number;
  /** Frames: the subtitles shown were left out (the player cannot draw them into a frame); the message says so. */
  withoutSubtitles?: boolean;
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

/** Every reason the plugin gives in its own words (English source text); the OSD shows them as text values. */
export const REASON = {
  burstLimits: "A burst needs 2–20 frames, an interval of 0.1–10 s and a known media duration.",
  burstPastEnd: "The burst runs past the end of the media. Use fewer frames or a shorter interval.",
  noName: "No free file name found.",
  unusable: "The current frame cannot be captured. Open a local video file first.",
  needDuration: "Bursts need a known media duration.",
  outOfRange: "The frame is past the end of the media.",
  changed: "The media changed; nothing was started.",
  busy: "The player is busy. Try again later.",
  unavailable: "The current frame cannot be captured.",
  captureFailed: "Could not capture the frame.",
  queueFull: "Five captures are still being saved. Please wait.",
  released: "The kept frame was released.",
  cancelled: "Cancelled.",
  clipboard: "Could not write to the clipboard",
  space: "Not enough disk space",
  exists: "The target file already exists",
  permission: "No write permission",
  encoder: "This frame cannot be encoded",
  exportFailed: "Could not create the image file",
  noOutput: "No snapshot saved yet.",
  cannotOpen: "Could not open the folder.",
  folderUnavailable: "The snapshot folder is unavailable. Please choose it again.",
  folderGone: "The file or folder was moved or deleted.",
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

/** A failure as data for the OSD: the plugin's reason, or (reason "") the player's or system's own message. */
export type Problem<R extends string = Reason> = { reason: R | ""; raw: string };
export const problemOf = (error: unknown): Problem =>
  error instanceof Failure ? { reason: error.code, raw: "" } : { reason: "", raw: message(error) };
/** A task failure: a job reason, or the message of anything else. */
export const jobProblem = (error: unknown): Problem<JobReason> =>
  error instanceof Failure && JOB_REASONS.includes(error.code) ? { reason: error.code as JobReason, raw: "" } : { reason: "", raw: message(error) };

export const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

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

/** Create the first unused folder `path`, `path (1)`, … and return it; an existing folder is never reused. */
export async function uniqueFolder(path: string) {
  const absolute = resolve(path);
  for (let i = 0; i < 1000; i++) {
    const candidate = i ? `${absolute} (${i})` : absolute;
    try {
      await mkdir(candidate);
      return candidate;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
  }
  throw new Failure("noName");
}

/** `1:02:03.450` style time for file names (no hours below one hour). */
export function clock(seconds: number, separator = ":") {
  const ms = Math.max(0, Math.round(seconds * 1000));
  const h = Math.floor(ms / 3_600_000), m = Math.floor(ms / 60_000) % 60, s = Math.floor(ms / 1000) % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return `${h ? `${h}${separator}` : ""}${two(m)}${separator}${two(s)}.${String(ms % 1000).padStart(3, "0")}`;
}

/** The source's file name without its extension (`capture` without one), safe in a file name. */
export function stemOf(source: string | null) {
  const path = source?.split(/[?#]/)[0] ?? "";
  const stem = path ? basename(path, extname(path)) : "";
  return (stem || "capture").replace(/[\\/:*?"<>|]/g, "_");
}

/** `movie_01.02.450.png`: source name plus media time (the default snapshot template). */
export function frameName(source: string | null, seconds: number, extension: string) {
  return `${stemOf(source)}_${clock(seconds, ".")}${extension}`;
}

export const usable = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.can.snapshot;

/** A local file path (not a URL). */
export const isLocal = (source: string | null): source is string =>
  !!source && isAbsolute(source) && !/^[a-z][a-z0-9+.-]*:\/\//i.test(source);

export function validate(spec: Spec, state: PlayerState) {
  if (!usable(state)) throw new Failure("unusable");
  const { duration } = state;
  if (spec.kind === "burst" && (duration === null || !Number.isFinite(duration))) throw new Failure("needDuration");
  const start = spec.current ? (state.position ?? 0) : spec.start;
  if (spec.kind === "burst") sampleTimes(start, spec.count, spec.step, duration!);
  if (duration !== null && start >= duration) throw new Failure("outOfRange");
}
