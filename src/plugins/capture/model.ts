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

export const KIND_LABEL: Record<Kind, string> = { image: "截图", burst: "连拍", clip: "片段" };

export const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function parseRange(start: unknown, end?: unknown) {
  const a = Number(String(start).trim());
  const b = end === undefined ? undefined : Number(String(end).trim());
  if (String(start).trim() === "" || !Number.isFinite(a) || a < 0 || (b !== undefined && (!Number.isFinite(b) || b <= a)))
    throw new Error("请输入有效的秒数，终点须晚于起点。");
  return { start: a, end: b };
}

export function sampleTimes(start: number, count: number, step: number, duration: number) {
  if (!Number.isInteger(count) || count < 2 || count > 20 || !Number.isFinite(step) || step < 0.1 || step > 10 || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(start) || start < 0)
    throw new Error("连拍需要 2–20 张、间隔 0.1–10 秒，且媒体时长已知。");
  const result = Array.from({ length: count }, (_, index) => start + index * step);
  if (result[result.length - 1]! >= duration) throw new Error("连拍超出媒体时长，请减少张数或间隔。");
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
  throw new Error("找不到可用的文件名。");
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
  if (!usable(state)) throw new Error("当前画面无法截取，请先打开本地视频文件。");
  const { duration } = state;
  if (spec.kind !== "image" && (duration === null || !Number.isFinite(duration))) throw new Error("连拍和导出片段需要已知的媒体时长。");
  const start = spec.current ? (state.position ?? 0) : spec.start;
  if (spec.kind === "burst") sampleTimes(start, spec.count, spec.step, duration!);
  if (duration !== null && (start >= duration || (spec.end !== undefined && spec.end > duration))) throw new Error("所选范围超出媒体时长。");
}

/** One source-internal audio track for a clip, or none. */
export function chooseAudio(spec: Spec, state: PlayerState): number | "none" {
  if (spec.kind !== "clip" || spec.track === "none") return "none";
  const tracks = state.audioTracks;
  const track = spec.track === "selected" ? tracks?.find((t) => t.selected) : tracks?.find((t) => t.id === Number(spec.track));
  if (!track && spec.track === "selected" && !tracks?.length) return "none";
  if (!track || !track.exportable) throw new Error("所选音轨无法导出，请换一条音轨或选择无声音。");
  return track.id;
}

/** Clip audio choices: index 0 none, 1 the selected track, then each track. */
export function audioChoices(state: PlayerState) {
  const tracks = state.audioTracks ?? [];
  return {
    labels: ["无声音", "当前音轨", ...tracks.map((t) => `音轨 ${t.id}${t.title || t.language ? ` · ${t.title ?? t.language}` : ""}`)],
    values: ["none", "selected", ...tracks.map((t) => String(t.id))],
  };
}
