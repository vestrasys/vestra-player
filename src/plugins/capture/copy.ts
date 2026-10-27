import { mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PlayerState, Scope, Snapshot } from "@vestra-player/sdk";
import { captureError } from "./form.ts";
import { Failure, usable } from "./model.ts";
import { jobError } from "./tasks.ts";

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
    if (!usable(seen)) throw new Failure("unusable");
    await mkdir(directory, { recursive: true });
    const file = join(directory, `clip-${++sequence}.png`);
    let snapshot: Snapshot | undefined;
    try {
      // The frame at the position read at the press (like Save), the host's current frame when none is known.
      const at = seen.position === null ? {} : { at: seen.position };
      snapshot = await plugin.media.capture({ ifCurrent: seen, processed, subtitles: processed && subtitles, ...at }).catch((error) => {
        throw captureError(error);
      });
      const job = await snapshot.export({ format: "png", destination: file });
      try {
        const state = await job.done;
        if (state.status === "failed")
          throw new Failure(state.outcome && "error" in state.outcome ? jobError(state.outcome.error) : "exportFailed");
        if (state.status !== "completed" || !state.outcome || !("destination" in state.outcome)) throw new Failure("cancelled");
        await plugin.clipboard.writeImage(state.outcome.destination).catch((error) => {
          console.error(`[capture] clipboard: ${(error as Error).message}`);
          throw new Failure("clipboard"); // shown without the "Snapshot failed" prefix
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
