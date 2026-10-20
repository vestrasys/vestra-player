import type { Scope, Window } from "../../sdk/plugin/index.ts";
import type { Queue } from "./queue.ts";

/** "添加链接…": adding only selects the row, it never plays. */
export function linkDialog(scope: Scope, queue: Queue): Window {
  const window = scope.windows.create("ui/link.slint", { id: "playlist-link" });
  let message = "输入 HTTP(S) 链接；添加只选择，不自动播放";
  window.bind(() => ({ message }));
  window.on("hide", () => window.hide());
  window.on("addLink", (url: string) => {
    try {
      queue.addUrl(url);
    } catch (error) {
      message = (error as Error).message;
      return;
    }
    message = "链接已添加，仅选中；显式播放需可用的来源提供者";
    return window.hide();
  });
  return window;
}
