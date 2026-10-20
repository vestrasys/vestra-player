import { mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PlayerState, Scope, Snapshot } from "../../sdk/plugin/index.ts";
import { captureError } from "./form.ts";
import { usable } from "./model.ts";
import { jobError } from "./tasks.ts";

/** The clipboard write failed (not the capture): shown without the 截图失败 prefix. */
export class ClipboardError extends Error {}

/**
 * F03: copy the current frame to the clipboard as PNG. Not an export task:
 * the snapshot is taken now (conditional on `seen`), exported into a private
 * temporary folder, put on the clipboard (`plugin.clipboard.writeImage`),
 * then the file and snapshot go.
 */
export function clipboard(plugin: Scope) {
  const directory = join(tmpdir(), `vestra-capture-${process.pid}`);
  let sequence = 0;

  async function copy(seen: PlayerState, processed: boolean, subtitles: boolean) {
    if (!usable(seen)) throw new Error("当前画面无法截取，请先打开本地视频文件。");
    await mkdir(directory, { recursive: true });
    const file = join(directory, `clip-${++sequence}.png`);
    let snapshot: Snapshot | undefined;
    try {
      snapshot = await plugin.media.capture({ ifCurrent: seen, processed, subtitles: processed && subtitles }).catch((error) => {
        throw captureError(error);
      });
      const job = await snapshot.export({ format: "png", destination: file });
      try {
        const state = await job.done;
        if (state.status === "failed")
          throw new Error(state.outcome && "error" in state.outcome ? jobError(state.outcome.error) : "导出失败");
        if (state.status !== "completed" || !state.outcome || !("destination" in state.outcome)) throw new Error("已取消。");
        await plugin.clipboard.writeImage(state.outcome.destination).catch((error) => {
          console.error(`[capture] clipboard: ${(error as Error).message}`);
          throw new ClipboardError("无法写入剪贴板");
        });
      } finally {
        await job.dispose();
      }
    } finally {
      await snapshot?.dispose();
      await rm(file, { force: true });
    }
  }

  return {
    copy,
    directory,
    /** Deactivation: empty the temporary folder. */
    stop: () => rm(directory, { recursive: true, force: true }),
  };
}
