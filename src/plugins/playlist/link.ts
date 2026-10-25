import type { Scope, Window } from "@vestra-player/sdk";
import { LinkError, type Queue, ROW_LIMIT } from "./queue.ts";

/**
 * "Add Link…" (`ui/link.slint`, the shared `UrlInputRow`): one link per
 * line, added in order; adding only selects the row, it never plays. A bad
 * line stops there with its reason (the lines before it are in the list,
 * adding them again only selects them) and the window stays open.
 */
export function linkDialog(scope: Scope, queue: Queue): Window {
  const window = scope.windows.create("ui/link.slint", { id: "playlist-link" });
  // What the message line says (`ui/link.slint` words it): the hint, why line `line`
  // (0: the only one) was not added, or that `count` links were added.
  let message: { notice: "hint" | "incomplete" | "refused" | "full" | "added"; line: number; count: number } = {
    notice: "hint",
    line: 0,
    count: 0,
  };
  window.bind(() => ({ ...message, limit: ROW_LIMIT }));
  window.on("cancel", () => window.hide());
  window.on("addLink", (text: string) => {
    const lines = String(text)
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line !== "");
    if (lines.length === 0) lines.push("");
    for (const [i, line] of lines.entries()) {
      try {
        queue.addUrl(line);
      } catch (error) {
        if (!(error instanceof LinkError)) throw error;
        message = { notice: error.code, line: lines.length > 1 ? i + 1 : 0, count: 0 };
        return;
      }
    }
    message = { notice: "added", line: 0, count: lines.length };
    return window.hide();
  });
  return window;
}
