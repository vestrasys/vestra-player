import type { ExportOptions, JobState, Plugin } from "@vestra-player/sdk";
import type { Encode, Method } from "./clip.ts";

/**
 * Where the host requests of the Snapshots & Clips proposal (§7) plug in. Each
 * one is a small function with a `TODO(R#)` that today does what SDK 1.6.8 and
 * player 0.7.6 can do, so the tab works now and degrades as the proposal says:
 *
 * | Request | Today | With it |
 * | --- | --- | --- |
 * | R1 stream copy | the Fast / Precise switch is not shown; clips are Precise | Fast is the default |
 * | R2 actual range | rows and OSD state the range asked for | the range the file holds |
 * | R3 range marks | nothing on the seek bar | the clip's range on the seek bar while the tab shows |
 * | R4 progress | a bar without a percentage and the time spent | percentage and time left |
 * | R5 OSD actions | "Clip exported as …" without a button | [Show in Folder] [Copy] |
 * | R6 encode options | Precise is today's encoding (High quality), no presets shown | presets and Advanced |
 * | R7 A-B points | Use A-B range needs the loop switched on | also with the loop off |
 */
export interface HostSupport {
  /** R1 (with R2): `Snapshot.export` can copy streams. */
  fastCut: boolean;
  /** R2: a finished job reports the times its file starts and ends. */
  actualRange: boolean;
  /** R3: the timeline takes a range mark. */
  rangeMarks: boolean;
  /** R4: a running job reports its progress. */
  progress: boolean;
  /** R5: `osd.show` takes buttons. */
  osdActions: boolean;
  /** R6: a Precise export takes encode options. */
  encodeOptions: boolean;
}

/** What the player offers. Tests may pretend to more (`activate(plugin, { support })`) to check the tab's views. */
export function hostSupport(_plugin: Plugin, overrides: Partial<HostSupport> = {}): HostSupport {
  return {
    // TODO(R1): detect stream-copy exports (SDK 1.6.9), for example a `copy` field of the mp4 export options.
    fastCut: false,
    // TODO(R2): true with the SDK that reports `outcome.start` / `outcome.end`.
    actualRange: false,
    // TODO(R3): true with the SDK whose `TimelineLayer.set` takes `kind: "range"`.
    rangeMarks: false,
    // TODO(R4): true with the SDK whose `JobState` has `progress`.
    progress: false,
    // TODO(R5): true with the SDK whose `osd.show` takes `actions`.
    osdActions: false,
    // TODO(R6): true with the SDK whose mp4 export takes encode options.
    encodeOptions: false,
    ...overrides,
  };
}

/**
 * The export options of a clip. Today every clip is today's re-encode (H.264
 * CRF 18 + AAC 192 kb/s in MP4), whatever `method` and `encode` say: the
 * callers only pass `fast` or a non-default encode when `HostSupport` says the
 * player takes them.
 */
export function clipExport(destination: string, end: number, _method: Method, _encode: Encode | undefined): ExportOptions {
  // TODO(R1): `_method === "fast"` → `{ format: "mp4", destination, end, copy: true }` (the player may change the extension to .mkv).
  // TODO(R6): Precise → add the encode options (`crf`, `preset` from `speed`, `maxHeight` from `height`, `fps`, `audioBitrate`).
  return { format: "mp4", destination, end };
}

/** The times the finished file holds. TODO(R2): read them from the outcome; today the range asked for. */
export function actualRange(_state: JobState, asked: { start: number; end: number }): { start: number; end: number } {
  return asked;
}

/** A running job's progress in seconds of media. TODO(R4): `state.progress`; today unknown (`null`). */
export function jobProgress(_state: JobState): { done: number; total: number } | null {
  return null;
}

/** A button of an OSD message (R5). */
export interface OsdAction {
  title: "Show in Folder" | "Copy";
  run: () => unknown;
}
/**
 * Show a finished capture's message. TODO(R5): pass `actions` (at most 2) as
 * `osd.show(text, args, { actions })`; today the message alone.
 */
export function showDone(plugin: Plugin, text: "Frame saved as {}" | "Clip exported as {}" | "Burst of {0} frames saved to {1}", args: (string | number)[], _actions: OsdAction[]) {
  void plugin.osd.show(text, args).catch(() => {});
}

/**
 * The clip's range on the seek bar while the tab shows (R3); `null` removes
 * it. TODO(R3): a timeline layer with one `kind: "range"` mark (and the
 * `timeline.marks` capability); today nothing is drawn (two point marks are
 * not a range, proposal §3.4).
 */
export function rangeMarker(_plugin: Plugin) {
  return {
    set(_range: { start: number; end: number } | null) {},
  };
}
