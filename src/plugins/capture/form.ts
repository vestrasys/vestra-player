import { mkdtemp, rmdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import type { PlayerState, Scope, Snapshot } from "@vestra-player/sdk";
import { chooseAudio, Failure, sampleTimes, uniqueFile, validate } from "./model.ts";
import type { Spec } from "./model.ts";
import type { Task, Tasks } from "./tasks.ts";

/** A host refusal of the snapshot as one of the plugin's reasons. */
export function captureError(error: unknown) {
  switch ((error as { code?: string }).code) {
    case undefined:
      return error;
    case "stale":
      return new Failure("changed");
    case "busy":
      return new Failure("busy");
    case "unavailable":
    case "unsupported":
      return new Failure("unavailable");
    default:
      console.error(`[capture] snapshot: ${(error as Error).message}`);
      return new Failure("captureFailed");
  }
}

/**
 * Freeze the input and queue the export. The snapshot is conditional on the
 * state the user acted on (`seen`), so a source change while a save panel was
 * open captures nothing. A retry reuses a retained snapshot when it has one.
 */
export function capturer(plugin: Scope, tasks: Tasks) {
  const { player } = plugin;
  let stopped = false;

  /** `destination`: the file for an image or clip, the parent of a new `Capture-*` folder for a burst. */
  async function start(spec: Spec, seen: PlayerState | null, destination: string, reuse?: Snapshot): Promise<Task> {
    if (tasks.active().length >= 5) throw new Failure("queueFull");
    let snapshot = reuse;
    let directory: string | undefined;
    try {
      let audio: number | "none" = "none";
      if (seen) {
        validate(spec, seen);
        audio = chooseAudio(spec, seen);
        await player.refresh();
        if (!player.isCurrent(seen)) throw new Failure("changed");
        snapshot = await plugin.media
          .capture({ ifCurrent: seen, processed: spec.processed, subtitles: spec.processed && spec.subtitles, audio, ...(spec.current ? {} : { at: spec.start }) })
          .catch((error) => {
            throw captureError(error);
          });
      }
      if (!snapshot) throw new Failure("expired");
      if (stopped) throw new Failure("cancelled");
      const frozen: Spec = { ...spec, start: snapshot.time, current: false };
      let target: string;
      if (frozen.kind === "burst") {
        sampleTimes(snapshot.time, frozen.count, frozen.step, snapshot.duration);
        target = directory = await mkdtemp(join(destination, "Capture-"));
      } else target = await uniqueFile(destination, new Set(tasks.list.map((task) => resolve(task.destination))));
      const task = tasks.add({ spec: frozen, sourceId: seen?.sourceId ?? 0, source: snapshot.source, snapshot, destination: target, directory });
      snapshot = directory = undefined;
      return task;
    } finally {
      await snapshot?.dispose();
      if (directory) await rmdir(directory).catch(() => {});
    }
  }

  /** Retry a failed or cancelled task: its retained input if still kept, otherwise the current source. */
  async function retry(task: Task) {
    const parent = task.spec.kind === "burst" ? dirname(task.destination) : task.destination;
    if (tasks.retained(task)) {
      const snapshot = tasks.take(task);
      return start({ ...task.spec }, null, parent, snapshot).then((next) => {
        next.sourceId = task.sourceId;
        return next;
      });
    }
    return start({ ...task.spec, current: false }, await player.refresh(), parent);
  }

  return {
    start,
    retry,
    stop() {
      stopped = true;
    },
  };
}
