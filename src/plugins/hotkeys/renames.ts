import type { BindingRef } from "@vestra-player/sdk";
import { refKey } from "./keys.ts";

/**
 * Commands whose full id (`"<manifest id>/<command id>"`) changed, as
 * `[old, new]`. A copy of `COMMAND_RENAMES` in src/host/src/command_renames.rs,
 * which is the source of truth (a plugin cannot import host code): add a pair
 * here whenever one is added there. The host moves the user's overrides and
 * palette recents itself; the commands recorded in ignore-defaults.json are
 * this plugin's to move (`renameRefs`).
 */
export const COMMAND_RENAMES: readonly (readonly [string, string])[] = [
  // Host 0.5 (core actions): playback-plugin commands the host now owns.
  ["vestra.playback/toggle-pause", "host/play-pause"],
  ["vestra.playback/close-source", "host/stop"],
  ["vestra.playback/previous", "host/previous"],
  ["vestra.playback/next", "host/next"],
  ["vestra.playback/backward-5", "host/seek-backward"],
  ["vestra.playback/forward-5", "host/seek-forward"],
  ["vestra.playback/rate-down", "host/rate-down"],
  ["vestra.playback/rate-up", "host/rate-up"],
  ["vestra.playback/rate-10", "host/rate-normal"],
  ["vestra.playback/volume-up", "host/volume-up"],
  ["vestra.playback/volume-down", "host/volume-down"],
  ["vestra.playback/mute", "host/mute"],
  // 2026-11-08: the A-B Loop moved from Chapters and Bookmarks (`vestra.marks`)
  // to Basic Playback (`vestra.playback`) with the same local ids.
  ...[
    "set-a", "clear-a", "a-back", "a-forward", "set-b", "clear-b", "b-back", "b-forward",
    "back", "forward", "segment", "toggle",
    "count.endless", "count.1", "count.2", "count.3", "count.5", "count.10",
  ].map((id) => [`vestra.marks/ab.${id}`, `vestra.playback/ab.${id}`] as const),
];

const RENAMED = new Map(COMMAND_RENAMES);

/**
 * The refs under their current ids, in order, each once (the first wins).
 * Unknown ids are kept; applying it again changes nothing.
 */
export function renameRefs(refs: readonly BindingRef[]): BindingRef[] {
  const seen = new Set<string>();
  const next: BindingRef[] = [];
  for (const ref of refs) {
    const global = RENAMED.get(refKey(ref)) ?? refKey(ref);
    if (seen.has(global)) continue;
    seen.add(global);
    const at = global.indexOf("/");
    next.push({ plugin: global.slice(0, at), command: global.slice(at + 1) });
  }
  return next;
}
