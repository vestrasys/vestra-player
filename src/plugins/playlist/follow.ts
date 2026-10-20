import { realpath } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import type { PlayerState, Scope } from "../../sdk/plugin/index.ts";
import { siblings } from "./media.ts";
import type { Queue } from "./queue.ts";

export type OpenMode = "only" | "similar" | "folder";

/**
 * A13: when another plugin opens a local file, add its similar files or its whole
 * folder and adopt the opened file's row, so next / auto-advance continue from it.
 * Each source is handled once; this plugin's own opens are never handled.
 */
export function follower(
  scope: Scope,
  queue: Queue,
  player: { busy(): boolean; owns(sourceId: number): boolean; adopt(rowId: string, state: PlayerState): void },
  mode: () => OpenMode,
  changed: () => void,
) {
  let handled = -1;

  async function handle(state: PlayerState) {
    const source = resolve(state.source!);
    const current = () => scope.player.state.sourceId === state.sourceId && !player.busy();
    let key = source;
    try {
      key = await realpath(source);
    } catch {
      /* the player reports the open failure */
    }
    const existing = queue.rows.find(
      (row) => row.kind === "file" && (row.key === key || row.source === source),
    );
    let paths: string[] = [];
    if (!existing) {
      try {
        paths = await siblings(source, mode() === "similar" ? "similar" : "folder");
      } catch {
        paths = [source]; // the folder is unreadable: add only this file, silently
      }
    }
    if (!current()) return;
    let target = existing;
    if (!target) {
      const result = await queue.addEntries(paths.map((path) => ({ kind: "file" as const, source: path })));
      if (!current()) return changed();
      target = result.rows[paths.indexOf(source)];
      if (result.added) queue.notice(`已加入同文件夹 ${result.added} 个文件`);
    }
    if (target) player.adopt(target.rowId, scope.player.state);
    changed();
  }

  /** Look at the current source (after every change and after this plugin's opens settle). */
  function check(state: PlayerState = scope.player.state) {
    if (state.sourceId === handled || player.busy()) return;
    handled = state.sourceId;
    if (player.owns(state.sourceId) || mode() === "only") return;
    if (state.kind !== "media" || !state.source || state.source.includes("://") || !isAbsolute(state.source)) return;
    if (state.phase === "empty") return;
    return handle(state);
  }
  return { check };
}
