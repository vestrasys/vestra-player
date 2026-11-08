import type { ExportOptions, JobState, Plugin, Text } from "@vestra-player/sdk";
import { exportEncoding } from "./clip.ts";
import type { Encode, Method } from "./clip.ts";

/**
 * The host requests of the Snapshots & Clips design (§7), in one place: each
 * is a small function over the API that answers it.
 *
 * | Request | API (SDK) | Here |
 * | --- | --- | --- |
 * | R1 stream copy | `format: "copy"` (1.6.10) | `clipExport` (Fast) |
 * | R2 actual range | `outcome.start` / `outcome.end` (1.6.10) | `actualRange` |
 * | R3 range marks | `TimelineLayer.setRanges` (1.6.9) | `rangeMarker` |
 * | R4 progress | `JobState.progress` (1.6.10) | `jobProgress` |
 * | R5 OSD actions | `osd.show(…, { actions })` (1.6.9) | `showDone` |
 * | R6 encode options | `ExportEncoding` (1.6.10) | `clipExport` (Precise), `clip.ts` `exportEncoding` |
 * | R7 A-B points | none yet | `clip.ts` `abRange` (needs the loop switched on) |
 *
 * All of them need player 0.7.6, which `minPlayerVersion` requires, so the
 * plugin uses them without asking (the SDK says not to probe by trying).
 */
export interface HostSupport {
  /** R1 (with R2): `Snapshot.export` can copy streams. */
  fastCut: boolean;
  /** R2: a finished clip reports the times its file starts and ends. */
  actualRange: boolean;
  /** R3: the timeline takes ranges. */
  rangeMarks: boolean;
  /** R4: a running clip reports its progress. */
  progress: boolean;
  /** R5: `osd.show` takes buttons. */
  osdActions: boolean;
  /** R6: a Precise export takes encode options. */
  encodeOptions: boolean;
}

/** What the player offers: everything, from player 0.7.6 on. Tests may turn parts off (`activate(plugin, { support })`). */
export function hostSupport(_plugin: Plugin, overrides: Partial<HostSupport> = {}): HostSupport {
  return { fastCut: true, actualRange: true, rangeMarks: true, progress: true, osdActions: true, encodeOptions: true, ...overrides };
}

/**
 * The export options of a clip. Fast: a stream copy from the keyframe at or
 * before the snapshot time; the player picks MP4 or MKV and gives the file
 * that extension. Precise: a re-encode with the tab's encoder settings
 * (`encode`; without them, the settings clips always had).
 */
export function clipExport(destination: string, end: number, method: Method, encode: Encode | undefined): ExportOptions {
  if (method === "fast") return { format: "copy", destination, end, container: "auto" };
  return encode ? { format: "mp4", destination, end, encoding: exportEncoding(encode) } : { format: "mp4", destination, end };
}

/** The times the finished file holds (R2): the player's `outcome.start` / `end`, the range asked for when it gives none. */
export function actualRange(state: JobState, asked: { start: number; end: number }): { start: number; end: number } {
  const outcome = state.outcome && "destination" in state.outcome ? state.outcome : null;
  const start = outcome?.start;
  const end = outcome?.end;
  return typeof start === "number" && typeof end === "number" && Number.isFinite(start) && Number.isFinite(end) && end > start ? { start, end } : asked;
}

/** A running clip's progress in seconds of the clip (R4); `null` while unknown (before writing starts, and for frames). */
export function jobProgress(state: JobState): { done: number; total: number } | null {
  const progress = state.progress;
  return progress && Number.isFinite(progress.done) && Number.isFinite(progress.total) && progress.total > 0
    ? { done: Math.min(progress.done, progress.total), total: progress.total }
    : null;
}

/** A button of a finished capture's message (R5): a label and one of the plugin's own commands. */
export interface OsdAction {
  /** Translated by the player like the text: a `plugin.i18n.text` value, so that extract finds it. */
  title: Text;
  command: "capture.revealLast" | "capture.copyLast" | "capture.tasks";
}
/**
 * Show a finished capture's message with its buttons (at most two). The
 * buttons run commands that act on the newest result; players before 0.7.6
 * would show the text alone, so every action is also in the tab and the
 * command palette.
 */
export function showDone(plugin: Plugin, text: "Frame saved as {}" | "Frame saved without subtitles as {}" | "Clip exported as {}" | "Burst of {0} frames saved to {1}", args: (string | number)[], actions: OsdAction[]) {
  // A refusal is logged: a "saved" message that does not show is otherwise invisible (GUI check 2026-11-08).
  void plugin.osd.show(text, args, { actions: actions.slice(0, 2) }).catch((error: Error) => console.error(`[capture] osd: ${error.message}`));
}

/**
 * The clip's range on the seek bar while the tab shows (R3), as a
 * `selection` range of this plugin's timeline layer; `null` removes it. The
 * same range is not sent twice; a new source clears the layer by itself.
 */
export function rangeMarker(plugin: Plugin, title: Text) {
  let layer: ReturnType<Plugin["timeline"]["layer"]> | null = null;
  let last = "none";
  return {
    set(range: { start: number; end: number } | null) {
      // Clearing reads no player state: it also runs when the tab goes to the background.
      if (!range) {
        if (layer && last !== "none") layer.setRanges([]);
        last = "none";
        return;
      }
      const state = plugin.player.state;
      const duration = state.duration ?? Infinity;
      const ranges = range && range.start < Math.min(range.end, duration)
        ? [{ start: Math.max(0, range.start), end: Math.min(range.end, duration), style: "selection" as const, title }]
        : [];
      const key = JSON.stringify([state.sourceId, ranges.map((r) => [r.start, r.end])]);
      if (key === last || (!layer && !ranges.length)) return;
      last = key;
      layer ??= plugin.timeline.layer();
      layer.setRanges(ranges, { ifCurrent: state });
    },
  };
}
