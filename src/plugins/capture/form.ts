import { mkdtemp, rmdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import type { PlayerState, Scope, Snapshot } from "../../sdk/plugin/index.ts";
import { chooseAudio, sampleTimes, uniqueFile, validate } from "./model.ts";
import type { Spec } from "./model.ts";
import type { Task, Tasks } from "./tasks.ts";

const CHANGED = "播放内容已变化，未开始。";

/** A host refusal of the snapshot in one Chinese sentence. */
function captureError(error: unknown) {
  switch ((error as { code?: string }).code) {
    case undefined:
      return error;
    case "stale":
      return new Error(CHANGED);
    case "busy":
      return new Error("播放器忙，请稍后再试。");
    case "unavailable":
    case "unsupported":
      return new Error("当前画面无法截取。");
    default:
      console.error(`[capture] snapshot: ${(error as Error).message}`);
      return new Error("截取画面失败。");
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
    if (tasks.active().length >= 5) throw new Error("已有一个导出在进行、四个在排队，请稍候。");
    let snapshot = reuse;
    let directory: string | undefined;
    try {
      let audio: number | "none" = "none";
      if (seen) {
        validate(spec, seen);
        audio = chooseAudio(spec, seen);
        await player.refresh();
        if (!player.isCurrent(seen)) throw new Error(CHANGED);
        snapshot = await plugin.media
          .capture({ ifCurrent: seen, processed: spec.processed, subtitles: spec.processed && spec.subtitles, audio, ...(spec.current ? {} : { at: spec.start }) })
          .catch((error) => {
            throw captureError(error);
          });
      }
      if (!snapshot) throw new Error("保留的画面已过期，请用当前画面重试。");
      if (stopped) throw new Error("已取消。");
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
