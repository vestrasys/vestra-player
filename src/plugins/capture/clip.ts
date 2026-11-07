import type { PlayerState } from "@vestra-player/sdk";
import { Failure } from "./model.ts";

/**
 * Pure clip rules of the Snapshots & Clips tab (proposal §3.3–§3.6): the two
 * cutting methods, A-B and "Last N s" ranges, keyframe snapping, the small
 * timeline, Precise presets and the estimates. No host calls here; the parts
 * that wait for host requests R1–R7 are in seams.ts.
 */

/** Fast: stream copy, starts on a keyframe (R1). Precise: re-encode from the exact frame (today's export). */
export type Method = "fast" | "precise";
export const METHODS: readonly Method[] = ["fast", "precise"];
/** The method a clip uses: the user's choice, or Precise while the player cannot copy streams. */
export const effectiveMethod = (chosen: Method, fastCut: boolean): Method => (fastCut ? chosen : "precise");

// ---------------------------------------------------------------- ranges
/** The lengths "Last N s" offers (proposal open question 4: adjustable, 30 s by default). */
export const LAST_SECONDS = [10, 30, 60, 120] as const;
export const lastSeconds = (value: unknown) => (LAST_SECONDS as readonly number[]).includes(Number(value)) ? Number(value) : 30;

/**
 * "Last N s": from N seconds before the position (not before 0) to the
 * position. `tooEarly` when less than 0.1 s has played.
 */
export function lastRange(position: number | null, seconds: number) {
  const end = position ?? 0;
  const start = Math.max(0, end - seconds);
  if (!Number.isFinite(end) || end - start < 0.1) throw new Failure("tooEarly");
  return { start, end };
}

/**
 * "Use A-B range": a one-time copy of the player's A-B loop (public
 * `PlayerState.loop`); `null` while there is none. TODO(R7): also read the A
 * and B points while the loop is switched off, once the player reports them.
 */
export function abRange(state: Pick<PlayerState, "loop">): { start: number; end: number } | null {
  const loop = state.loop;
  if (!loop || !Number.isFinite(loop.a) || !Number.isFinite(loop.b) || loop.b <= loop.a) return null;
  return { start: loop.a, end: loop.b };
}

/** Export Clip… on a fresh tab: from the position, 10 s (shorter at the end of the media). */
export function defaultRange(position: number | null, duration: number | null) {
  const start = position ?? 0;
  const end = Math.min(start + 10, duration ?? Infinity);
  return { start, end: end > start ? end : start + 10 };
}

// ---------------------------------------------------------------- keyframes (Fast)
/**
 * ffprobe's arguments for the video key packets around a window (no decoding:
 * packet flags only). `-read_intervals` seeks to the keyframe at or before
 * `from`, so the keyframe a Fast cut starts on is always included.
 */
export const keyframeArgs = (path: string, from: number, to: number) => [
  "-v", "error",
  "-select_streams", "v:0",
  "-read_intervals", `${Math.max(0, from).toFixed(3)}%${Math.max(from, to).toFixed(3)}`,
  "-show_entries", "packet=pts_time,flags",
  "-of", "csv=p=0",
  path,
];

/** The times of the key packets in ffprobe's `pts_time,flags` lines, sorted, without repeats. */
export function parseKeyframes(stdout: string): number[] {
  const times = new Set<number>();
  for (const line of stdout.split(/\r?\n/)) {
    const [time, flags] = line.split(",");
    const seconds = Number(time);
    if (time && flags?.includes("K") && Number.isFinite(seconds) && seconds >= 0) times.add(Math.round(seconds * 1000) / 1000);
  }
  return [...times].sort((a, b) => a - b);
}

/** The keyframe a Fast cut starts on: the last at or before `start`; `null` when none is known. */
export function snapStart(keyframes: readonly number[], start: number): number | null {
  let found: number | null = null;
  for (const time of keyframes) if (time <= start + 0.0005) found = time;
  return found;
}

/** ffprobe's arguments for the codec of the first video stream (the Fast container preview). */
export const codecArgs = (path: string) => ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of", "csv=p=0", path];

const MP4_VIDEO = new Set(["h264", "hevc", "av1", "vp9", "mpeg4"]);
const MP4_AUDIO = new Set(["aac", "mp3", "alac", "opus", "flac", "ac3", "eac3"]);
/**
 * The container a Fast cut writes (proposal §3.3): MP4 when the video and the
 * chosen audio fit in it, MKV otherwise; `""` while the video codec is unknown.
 * The player decides in the end (R1) and reports the file it wrote.
 */
export function fastContainer(video: string | null, audio: string | null): "mp4" | "mkv" | "" {
  if (!video) return "";
  return MP4_VIDEO.has(video.toLowerCase()) && (!audio || MP4_AUDIO.has(audio.toLowerCase())) ? "mp4" : "mkv";
}

// ---------------------------------------------------------------- the small timeline
export type Timeline = {
  shown: boolean;
  /** Ratios 0–1 of the window: the range typed, the range written, the playhead (-1 outside), the keyframe used (-1 none). */
  wanta: number;
  wantb: number;
  gota: number;
  gotb: number;
  head: number;
  key: number;
  /** The keyframe ticks as Path commands in a 1000 × 10 viewbox ("" without any). */
  ticks: string;
};
export const NO_TIMELINE: Timeline = { shown: false, wanta: 0, wantb: 0, gota: 0, gotb: 0, head: -1, key: -1, ticks: "" };

/** The window the small timeline shows: about 30 s around the clip, inside the media. */
export function timelineWindow(start: number, end: number, duration: number | null) {
  const length = end - start;
  const pad = Math.max(2, (30 - length) / 2);
  const from = Math.max(0, start - pad);
  const to = Math.min(duration ?? Infinity, end + pad);
  return { from, to: to > from ? to : from + 1 };
}

/** The small timeline (proposal §3.2): what was typed, what will be written, keyframes and the playhead. */
export function timeline(range: { start: number; end: number }, actual: { start: number; end: number }, keyframes: readonly number[], playhead: number | null, duration: number | null, used: number | null): Timeline {
  const { from, to } = timelineWindow(Math.min(range.start, actual.start), Math.max(range.end, actual.end), duration);
  const at = (time: number) => Math.min(1, Math.max(0, (time - from) / (to - from)));
  const round = (ratio: number) => Math.round(ratio * 10000) / 10000;
  const ticks = keyframes
    .filter((time) => time >= from && time <= to)
    .map((time) => `M ${Math.round(at(time) * 1000)} 0 V 10`)
    .join(" ");
  return {
    shown: true,
    wanta: round(at(range.start)),
    wantb: round(at(range.end)),
    gota: round(at(actual.start)),
    gotb: round(at(actual.end)),
    head: playhead === null || playhead < from || playhead > to ? -1 : round(at(playhead)),
    key: used === null ? -1 : round(at(used)),
    ticks,
  };
}

// ---------------------------------------------------------------- Precise quality (R6)
/** The encode options of a Precise cut (request R6): finite choices, never free FFmpeg arguments. */
export interface Encode {
  /** 14–30: lower is a better picture and a bigger file. */
  crf: number;
  speed: "faster" | "balanced" | "smaller";
  /** 0 = the source's height. */
  height: 0 | 1080 | 720 | 480;
  /** 0 = the source's frame rate. */
  fps: 0 | 60 | 30;
  audio: 128 | 192 | 256;
}
export type Preset = "source" | "high" | "small" | "custom";
export const PRESETS: readonly Exclude<Preset, "custom">[] = ["source", "high", "small"];
/**
 * `high` is today's encoding (CRF 18, x264 veryfast, AAC 192 kb/s, the
 * source's size and rate), so the default output does not change when R6
 * lands. `source` aims at the source's quality; `small` caps at 1080p.
 */
export const PRESET_ENCODE: Readonly<Record<Exclude<Preset, "custom">, Encode>> = {
  source: { crf: 16, speed: "balanced", height: 0, fps: 0, audio: 256 },
  high: { crf: 18, speed: "faster", height: 0, fps: 0, audio: 192 },
  small: { crf: 26, speed: "balanced", height: 1080, fps: 0, audio: 128 },
};
export const SPEEDS: readonly Encode["speed"][] = ["faster", "balanced", "smaller"];
export const HEIGHTS: readonly Encode["height"][] = [0, 1080, 720, 480];
export const RATES: readonly Encode["fps"][] = [0, 60, 30];
export const AUDIO_RATES: readonly Encode["audio"][] = [128, 192, 256];
export const crfOf = (value: unknown) => {
  const n = Math.round(Number(value));
  return Number.isFinite(n) ? Math.min(30, Math.max(14, n)) : 18;
};
/** Any valid encode options from stored data, field by field; High quality's for the rest. */
export function encodeOf(value: unknown): Encode {
  const v = (typeof value === "object" && value !== null ? value : {}) as Partial<Record<keyof Encode, unknown>>;
  const high = PRESET_ENCODE.high;
  const pick = <T>(list: readonly T[], item: unknown, fallback: T) => (list.includes(item as T) ? (item as T) : fallback);
  return {
    crf: v.crf === undefined ? high.crf : crfOf(v.crf),
    speed: pick(SPEEDS, v.speed, high.speed),
    height: pick(HEIGHTS, v.height, high.height),
    fps: pick(RATES, v.fps, high.fps),
    audio: pick(AUDIO_RATES, v.audio, high.audio),
  };
}
/** The preset whose options these are, `custom` when none (the Quality box then says Custom). */
export function presetOf(encode: Encode): Preset {
  return PRESETS.find((preset) => JSON.stringify(PRESET_ENCODE[preset]) === JSON.stringify(encode)) ?? "custom";
}

// ---------------------------------------------------------------- estimates
export interface Estimate {
  /** Seconds of media. */
  length: number;
  /** Megabytes, a range (equal ends when it is one number); 0 when unknown. */
  sizelow: number;
  sizehigh: number;
  /** Seconds of work, a range; 0 when unknown. */
  timelow: number;
  timehigh: number;
}
/** A conservative disk speed for stream copies, bytes per second. */
const COPY_SPEED = 50e6;
/**
 * Size and time of a clip (proposal §3.5), all approximate ("≈"):
 * - Fast: the source's share of its file size; time from a slow disk.
 * - Precise: a range around the source's average rate, scaled by the
 *   quality number (each 6 CRF about halves the size) and the height cap;
 *   time from the last Precise export measured on this machine (seconds of
 *   work per second of media), else a wide range.
 */
export function estimate(input: { method: Method; length: number; fileBytes: number | null; duration: number | null; encode: Encode; secondsPerSecond: number | null; height: number | null }): Estimate {
  const { method, length, fileBytes, duration, encode, secondsPerSecond, height } = input;
  const share = fileBytes && duration && duration > 0 ? (fileBytes * Math.min(length, duration)) / duration : null;
  const mb = (bytes: number) => Math.max(0.1, Math.round((bytes / 1e6) * 10) / 10);
  if (method === "fast") {
    const time = share === null ? 0 : Math.max(1, Math.ceil(share / COPY_SPEED));
    return { length, sizelow: share === null ? 0 : mb(share), sizehigh: share === null ? 0 : mb(share), timelow: time, timehigh: time };
  }
  let sizelow = 0, sizehigh = 0;
  if (share !== null) {
    const quality = Math.pow(2, (18 - encode.crf) / 6);
    const scale = encode.height && height && height > encode.height ? Math.pow(encode.height / height, 2) : 1;
    sizelow = mb(share * 0.7 * quality * scale);
    sizehigh = mb(share * 1.3 * quality * scale);
  }
  const timelow = secondsPerSecond ? Math.max(1, Math.round(length * secondsPerSecond * 0.8)) : Math.max(1, Math.round(length * 0.3));
  const timehigh = secondsPerSecond ? Math.max(timelow, Math.round(length * secondsPerSecond * 1.25)) : Math.max(timelow, Math.round(length * 3));
  return { length, sizelow, sizehigh, timelow, timehigh };
}
