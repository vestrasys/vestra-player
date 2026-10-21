import type { Scope, Window } from "../../sdk/plugin/index.ts";
import type { Queue } from "./queue.ts";

const HINT = "输入 HTTP(S) 链接；添加只选择，不自动播放";

/**
 * "添加链接…" (`ui/link.slint`, the shared `UrlInputRow`): one link per
 * line, added in order; adding only selects the row, it never plays. A bad
 * line stops there with its reason (the lines before it are in the list,
 * adding them again only selects them) and the window stays open.
 */
export function linkDialog(scope: Scope, queue: Queue): Window {
  const window = scope.windows.create("ui/link.slint", { id: "playlist-link" });
  let message = HINT;
  window.bind(() => ({ message }));
  window.on("hide", () => window.hide());
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
        const reason = (error as Error).message;
        message = lines.length > 1 ? `第 ${i + 1} 行：${reason}` : reason;
        return;
      }
    }
    message =
      lines.length > 1
        ? `已添加 ${lines.length} 个链接，仅选中；显式播放需可用的来源提供者`
        : "链接已添加，仅选中；显式播放需可用的来源提供者";
    return window.hide();
  });
  return window;
}
