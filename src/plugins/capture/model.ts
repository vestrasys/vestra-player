import { lstat } from "node:fs/promises";
import { basename, dirname, extname, join, resolve } from "node:path";
import type { Json, PlayerState } from "../../sdk/plugin/index.ts";

/** Pure capture rules: form parsing, validation, ranges and file naming. */
export type Kind = "image" | "burst" | "clip";
export type Section = "Screenshot" | "Burst" | "Video clip" | "Export tasks";
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
}

export const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function parseRange(start: unknown, end?: unknown) {
  const a = Number(String(start).trim());
  const b = end === undefined ? undefined : Number(String(end).trim());
  if (String(start).trim() === "" || !Number.isFinite(a) || a < 0 || (b !== undefined && (!Number.isFinite(b) || b <= a)))
    throw new Error("Enter valid seconds; end must be after start.");
  return { start: a, end: b };
}

export function sampleTimes(start: number, count: number, step: number, duration: number) {
  if (!Number.isInteger(count) || count < 2 || count > 20 || !Number.isFinite(step) || step < 0.1 || step > 10 || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(start) || start < 0)
    throw new Error("Burst requires 2–20 images, 0.1–10s spacing and a finite source duration.");
  const result = Array.from({ length: count }, (_, index) => start + index * step);
  if (result[result.length - 1]! >= duration) throw new Error("Burst exceeds source duration. Reduce count or spacing.");
  return result;
}

/** The first unused `name (n).ext` next to `path`; existing files are never chosen. */
export async function uniqueFile(path: string) {
  const absolute = resolve(path);
  const extension = extname(absolute);
  const stem = basename(absolute, extension);
  for (let i = 0; i < 1000; i++) {
    const candidate = i ? join(dirname(absolute), `${stem} (${i})${extension}`) : absolute;
    try {
      await lstat(candidate);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return candidate;
      throw error;
    }
  }
  throw new Error("No available filename among the next 1000 suffixes.");
}

/** Callback arguments, in the order `capture.slint` passes them. */
export function parseSpec(kind: Kind, args: Json[]): Spec {
  const range = parseRange(args[0], kind === "clip" ? args[1] : undefined);
  return {
    kind,
    start: range.start,
    end: range.end,
    count: kind === "burst" ? Number(args[1]) : 1,
    step: kind === "burst" ? Number(args[2]) : 1,
    processed: args[kind === "burst" ? 3 : kind === "clip" ? 2 : 1] === true,
    subtitles: args[kind === "burst" ? 4 : kind === "clip" ? 4 : 2] === true,
    track: kind === "clip" ? String(args[3] ?? "none").trim() : "none",
    current: kind !== "clip" && args[kind === "burst" ? 5 : 3] === true,
  };
}

/** The arguments that recreate `spec` from the form ("retry current source"). */
export function argsOf(spec: Spec): Json[] {
  if (spec.kind === "clip") return [String(spec.start), String(spec.end), spec.processed, spec.track, spec.subtitles];
  if (spec.kind === "burst") return [String(spec.start), String(spec.count), String(spec.step), spec.processed, spec.subtitles, false];
  return [String(spec.start), spec.processed, spec.subtitles, false];
}

export const usable = (state: PlayerState) =>
  state.kind === "media" && (state.phase === "ready" || state.phase === "ended") && state.can.snapshot;

export function validate(spec: Spec, state: PlayerState) {
  if (!usable(state)) throw new Error("This source cannot be captured. Open a finite video file.");
  const { duration } = state;
  if (spec.kind !== "image" && (duration === null || !Number.isFinite(duration))) throw new Error("Burst and clip export require known finite duration.");
  const start = spec.current ? (state.position ?? 0) : spec.start;
  if (spec.kind === "burst") sampleTimes(start, spec.count, spec.step, duration!);
  if (duration !== null && (start >= duration || (spec.end !== undefined && spec.end > duration))) throw new Error("The selected range exceeds the source duration.");
}

/** One source-internal audio track for a clip, or none. */
export function chooseAudio(spec: Spec, state: PlayerState): number | "none" {
  if (spec.kind !== "clip" || spec.track === "none") return "none";
  const tracks = state.audioTracks;
  const track = spec.track === "selected" ? tracks?.find((t) => t.selected) : tracks?.find((t) => t.id === Number(spec.track));
  if (!track && spec.track === "selected" && !tracks?.length) return "none";
  if (!track || !track.exportable) throw new Error("Choose a source internal audio track ID or none.");
  return track.id;
}

export const sourceLabel = (state: PlayerState) =>
  usable(state) ? `${state.source} · generation ${state.sourceId}` : "Open a finite, independently readable video source.";

export const trackList = (state: PlayerState) =>
  state.audioTracks?.map((t) => `${t.id}: ${t.title ?? t.language ?? t.codec ?? "audio"}${t.selected ? " (selected)" : ""}`).join("; ") || "No audio tracks";
