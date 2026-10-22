import { realpath } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import type { PlayerState, Scope } from "@vestra-player/sdk";
import { SCAN_LIMIT, siblings } from "./media.ts";
import type { Queue } from "./queue.ts";

export type OpenMode = "only" | "similar" | "folder";

/**
 * A13 / IA-PL-01: when another plugin opens a local file (append off), the list
 * becomes: cleared, then the file with its similar files or its whole folder (`media.ts` `siblings`), the opened file's row
 * adopted so next / auto-advance continue from it. With 「打开或拖入时追加」
 * (`append`) the old rule stays: add without duplicates, and 「仅打开该文件」
 * leaves the list alone. Each source is handled once; this plugin's own opens never.
 */
export function follower(
  scope: Scope,
  queue: Queue,
  player: { busy(): boolean; owns(sourceId: number): boolean; adopt(rowId: string, state: PlayerState): void },
  mode: () => OpenMode,
  append: () => boolean,
  changed: () => void,
) {
  let handled = -1;
  let first = true;

  async function handle(state: PlayerState, initial: boolean) {
    const source = resolve(state.source!);
    const current = () => scope.player.state.sourceId === state.sourceId && !player.busy();
    let key = source;
    try {
      key = await realpath(source);
    } catch {
      /* the player reports the open failure */
    }
    // The source playing when this plugin starts may be a row of the restored list
    // (this plugin was restarted): adopt it, never clear for it.
    const replace = !append() && !initial;
    const existing = replace
      ? undefined
      : queue.rows.find((row) => row.kind === "file" && (row.key === key || row.source === source));
    let paths: string[] = [source];
    let truncated = false;
    if (!existing && mode() !== "only") {
      try {
        ({ paths, truncated } = await siblings(source, mode() === "similar" ? "similar" : "folder"));
      } catch {
        /* the folder is unreadable: only this file, silently */
      }
    }
    if (!current()) return;
    let target = existing;
    if (!target) {
      if (replace) queue.clear(false);
      const result = await queue.addEntries(paths.map((path) => ({ kind: "file" as const, source: path })));
      if (!current()) return changed();
      target = result.rows[paths.indexOf(source)];
      const others = replace ? result.added - 1 : result.added;
      const limit = truncated ? `，已达上限 ${SCAN_LIMIT}` : "";
      if (others > 0) queue.notice(`已加入同文件夹 ${others} 个文件${limit}`);
    }
    if (target) player.adopt(target.rowId, scope.player.state);
    changed();
  }

  /** Look at the current source (after every change and after this plugin's opens settle). */
  function check(state: PlayerState = scope.player.state) {
    const initial = first;
    first = false;
    if (state.sourceId === handled || player.busy()) return;
    handled = state.sourceId;
    if (player.owns(state.sourceId)) return;
    if (state.kind !== "media" || !state.source || state.source.includes("://") || !isAbsolute(state.source)) return;
    if (state.phase === "empty") return;
    // Appending in 「仅打开该文件」 mode: the list is not touched (UI-PL-20).
    if (mode() === "only" && (append() || initial)) return;
    return handle(state, initial);
  }
  return { check };
}
