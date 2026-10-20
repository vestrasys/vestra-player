import type { Plugin } from "../../sdk/plugin/index.ts";
import { captureForm } from "./form.ts";
import { argsOf, message, sourceLabel, trackList } from "./model.ts";
import type { Section } from "./model.ts";
import { Tasks, terminal } from "./tasks.ts";
import type { Task } from "./tasks.ts";

const ENTRIES = [
  ["capture.still", "保存当前画面…", "Screenshot", 321],
  ["capture.burst", "连拍…", "Burst", 324],
  ["capture.clip", "导出片段…", "Video clip", 322],
  ["capture.tasks", "查看导出任务…", "Export tasks", 323],
] as const;

/** Capture and export: one window, four menu entrances, a queue of frozen exports. */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const view = {
    status: "Choose capture options. Exports use independent offline decoding.",
    formBusy: false,
    confirmation: "",
    confirming: false,
  };
  const say = (text: string) => {
    view.status = text;
    plugin.update();
  };
  const tasks = new Tasks(plugin, say);
  const form = captureForm(plugin, view, tasks);
  const window = plugin.windows.create("capture.slint");
  const errorWindow = plugin.windows.create("capture-error.slint");

  window.bind(() => ({
    ...view,
    sourceLabel: sourceLabel(player.state),
    tracks: trackList(player.state),
    queueCount: `${tasks.active().length}/5 accepted · one running, up to four queued`,
    taskRows: tasks.rows(),
  }));
  window.on("exportImage", (start: string, processed: boolean, subtitles: boolean, current: boolean) => {
    return form.submit("image", [start, processed, subtitles, current]);
  });
  window.on("exportBurst", (start: string, count: string, step: string, processed: boolean, subtitles: boolean, current: boolean) => {
    return form.submit("burst", [start, count, step, processed, subtitles, current]);
  });
  window.on("exportClip", (start: string, end: string, processed: boolean, track: string, subtitles: boolean) => {
    return form.submit("clip", [start, end, processed, track, subtitles]);
  });
  window.on("confirmStart", () => form.answer(true));
  window.on("rejectStart", () => form.answer(false));
  window.on("cancelForm", () => form.cancel());
  // In-out fields (section, start/end time) belong to the user once shown; the
  // plugin writes them only here and in open(), with set().
  window.on("markStart", async () => {
    if (!view.formBusy) window.set({ startTime: String((await player.refresh()).position ?? 0) });
  });
  window.on("markEnd", async () => {
    if (!view.formBusy) window.set({ endTime: String((await player.refresh()).position ?? 0) });
  });
  const onTask = (name: string, action: (task: Task) => unknown) =>
    window.on(name, (id: string) => {
      const task = tasks.find(String(id));
      return task && action(task);
    });
  onTask("cancelTask", (task) => tasks.cancel(task));
  onTask("queryTask", async (task) => say(await tasks.query(task)));
  onTask("openOutput", (task) => tasks.open(task));
  onTask("retryRetained", (task) => {
    if (!terminal(task) || task.status === "completed") throw new Error("Only a failed/cancelled task can be retried.");
    return form.submit(task.spec.kind, [], task);
  });
  onTask("retryCurrent", (task) => form.submit(task.spec.kind, argsOf(task.spec)));
  plugin.onError((error) => say(error.message));

  async function open(section: Section) {
    if (!window.opened) {
      // A fresh window starts its range at the current position.
      const { position, duration } = await player.refresh();
      window.set({
        startTime: String(position ?? 0),
        endTime: String(Math.min((position ?? 0) + 10, duration ?? Infinity)),
      });
    }
    window.set({ section });
    await window.show();
    if (errorWindow.opened) await errorWindow.close();
  }
  // The capture window could not open: show the reason in a separate plain window.
  async function reportOpenError(error: unknown) {
    const detail = `Cannot open Capture and export: ${message(error)}. Close this message and choose the capture menu again to retry.`;
    console.error(`[capture] ${detail}`);
    try {
      errorWindow.set({ message: detail });
      await errorWindow.show();
    } catch (feedbackError) {
      console.error(`[capture] Error window also failed: ${message(feedbackError)}`);
      if (window.opened) say(detail);
    }
  }
  for (const [id, title, section, order] of ENTRIES)
    plugin.commands.add({ id, path: ["视频", "截图与导出", title], order, run: () => open(section).catch(reportOpenError) });

  // Closing the window keeps tasks running; deactivation cancels them (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    form.stop();
    await tasks.stop();
  });
}
