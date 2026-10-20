import { existsSync } from "node:fs";
import { mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import type { Plugin, Window } from "../../sdk/plugin/index.ts";
import { capturer } from "./form.ts";
import { audioChoices, frameName, message, parseRange, usable } from "./model.ts";
import type { Spec } from "./model.ts";
import { reveal, Tasks, terminal } from "./tasks.ts";
import type { Task } from "./tasks.ts";

const defaultFolder = () => {
  const pictures = join(homedir(), "Pictures");
  return existsSync(pictures) ? pictures : homedir();
};
/** The capture folder kept in `settings.json`; a missing or unreadable file means the default. */
async function loadFolder(dataDirectory: string) {
  try {
    const { folder } = JSON.parse(await readFile(join(dataDirectory, "settings.json"), "utf8")) as { folder?: unknown };
    if (typeof folder === "string" && folder && (await stat(folder)).isDirectory()) return folder;
  } catch {
    // first start, removed folder or damaged file: fall back to the default
  }
  return defaultFolder();
}
/** Write a temporary file, then rename it over `settings.json`, so a crash never leaves half a file. */
async function saveFolder(dataDirectory: string, folder: string) {
  const file = join(dataDirectory, "settings.json");
  const temporary = `${file}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify({ folder }, null, 2)}\n`);
  await rename(temporary, file);
}
/** An OSD line is at most 200 characters: fall back to the file name for long paths. */
const fit = (prefix: string, path: string) =>
  (prefix + path).length <= 200 ? prefix + path : (prefix + basename(path)).slice(0, 200);

/**
 * Capture (decision D3): 保存当前画面 saves straight into the capture folder,
 * 另存为 asks for a file, 连拍 / 导出片段 / 导出任务 are their own dialogs.
 */
export default async function activate(plugin: Plugin) {
  const { player, osd } = plugin;
  const data = (() => {
    try {
      return plugin.dataDirectory;
    } catch {
      return null; // no data directory: the folder is kept for this session only
    }
  })();
  let folder = data ? await loadFolder(data) : defaultFolder();
  const ui = { burstBusy: false, clipBusy: false, stillBusy: false, burstStatus: "", clipStatus: "", tasksStatus: "" };
  const say = (text: string) => void osd.show(text.slice(0, 200)).catch(() => {});
  const tasks = new Tasks(plugin, finished);
  const exporter = capturer(plugin, tasks);
  const burst = plugin.windows.create("ui/burst.slint");
  const clip = plugin.windows.create("ui/clip.slint");
  const list = plugin.windows.create("ui/tasks.slint");
  const errorWindow = plugin.windows.create("ui/error.slint");

  /** A task reached its end: report it on screen and in its dialog. */
  function finished(task: Task) {
    const path = task.outputs[0]?.path ?? task.destination;
    if (task.status === "completed") {
      if (task.spec.kind === "image") say(fit("帧已保存为 ", path));
      else if (task.spec.kind === "burst") say(fit(`连拍已保存 ${task.outputs.length} 张到 `, task.destination));
      else say(fit("片段已导出为 ", path));
      if (task.spec.kind === "burst") ui.burstStatus = `已保存 ${task.outputs.length} 张`;
      if (task.spec.kind === "clip") ui.clipStatus = `已导出 ${basename(path)}`;
    } else if (task.status === "failed") {
      const text = `${task.spec.kind === "image" ? "截图" : task.spec.kind === "burst" ? "连拍" : "导出"}失败：${task.error}`;
      say(text);
      if (task.spec.kind === "burst") ui.burstStatus = text;
      if (task.spec.kind === "clip") ui.clipStatus = text;
    }
  }

  // ---------------------------------------------------------------- single frames
  const frameSpec = (seen: { position: number | null }, current: boolean): Spec => ({
    kind: "image", start: seen.position ?? 0, count: 1, step: 1, processed: false, subtitles: false, track: "none", current,
  });
  /** Ctrl+E: the raw source frame now, into the capture folder, no dialogs. */
  async function saveFrame() {
    if (ui.stillBusy) return;
    ui.stillBusy = true;
    try {
      const seen = await player.refresh();
      await mkdir(folder, { recursive: true }).catch(() => {
        throw new Error("截图文件夹不可用，请重新选择。");
      });
      await exporter.start(frameSpec(seen, true), seen, join(folder, frameName(seen.source, seen.position ?? 0, ".png")));
      if (tasks.active().length > 1) say("截图已排队");
    } catch (error) {
      say(`截图失败：${message(error)}`);
    } finally {
      ui.stillBusy = false;
    }
  }
  /** The frame shown when the command ran, into a file the user picks. */
  async function saveFrameAs() {
    if (ui.stillBusy) return;
    ui.stillBusy = true;
    try {
      const seen = await player.refresh();
      const spec = frameSpec(seen, false);
      if (!usable(seen)) throw new Error("当前画面无法截取，请先打开本地视频文件。");
      const chosen = await plugin.dialogs.saveFile({ title: "当前画面另存为", fileName: frameName(seen.source, spec.start, ".png") });
      if (!chosen) return;
      await exporter.start(spec, seen, /\.png$/i.test(chosen) ? chosen : `${chosen}.png`);
    } catch (error) {
      say(`截图失败：${message(error)}`);
    } finally {
      ui.stillBusy = false;
    }
  }
  async function chooseFolder() {
    const chosen = await plugin.dialogs.openDirectory({ title: "选择截图文件夹" });
    if (!chosen) return;
    if (!(await stat(chosen).catch(() => null))?.isDirectory()) return say("请选择一个已有的文件夹。");
    folder = chosen;
    if (data) await saveFolder(data, folder).catch((error) => console.error(`[capture] cannot save settings: ${message(error)}`));
    say(fit("截图文件夹：", folder));
  }
  const openFolder = () => mkdir(folder, { recursive: true }).then(() => reveal(folder));

  // ---------------------------------------------------------------- 连拍
  burst.bind(() => {
    const last = tasks.latest("burst");
    return {
      folder,
      busy: ui.burstBusy,
      canCapture: usable(player.state),
      progress: last ? tasks.progress(last) : "",
      status: ui.burstStatus,
    };
  });
  burst.on("pickFolder", chooseFolder);
  burst.on("openFolder", openFolder);
  burst.on("captureFrame", saveFrame);
  burst.on("markStart", async () => burst.set({ startTime: String((await player.refresh()).position ?? 0) }));
  burst.on("startBurst", async (start: string, count: string, step: string, current: boolean, processed: boolean, subtitles: boolean) => {
    if (ui.burstBusy) return;
    ui.burstBusy = true; // exclusion before the first await
    try {
      const spec: Spec = {
        kind: "burst", start: current ? 0 : parseRange(start).start, count: Number(count), step: Number(step),
        processed: processed === true, subtitles: subtitles === true, track: "none", current: current === true,
      };
      const seen = await player.refresh();
      await mkdir(folder, { recursive: true });
      await exporter.start(spec, seen, folder);
      ui.burstStatus = `已开始连拍 ${spec.count} 张`;
    } catch (error) {
      ui.burstStatus = message(error);
    } finally {
      ui.burstBusy = false;
    }
  });

  // ---------------------------------------------------------------- 导出片段
  clip.bind(() => ({
    busy: ui.clipBusy,
    canExport: usable(player.state),
    audioLabels: audioChoices(player.state).labels,
    status: ui.clipStatus,
  }));
  const mark = (field: "startTime" | "endTime") => async () => {
    if (!ui.clipBusy) clip.set({ [field]: String((await player.refresh()).position ?? 0) });
  };
  clip.on("markStart", mark("startTime"));
  clip.on("markEnd", mark("endTime"));
  clip.on("openTasks", () => show(list));
  clip.on("exportClip", async (start: string, end: string, processed: boolean, subtitles: boolean, audio: number) => {
    if (ui.clipBusy) return;
    ui.clipBusy = true; // exclusion before the first await
    try {
      const range = parseRange(start, end);
      const seen = await player.refresh();
      const spec: Spec = {
        kind: "clip", start: range.start, end: range.end, count: 1, step: 1,
        processed: processed === true, subtitles: subtitles === true,
        track: audioChoices(seen).values[Number(audio)] ?? "none", current: false,
      };
      if (!usable(seen)) throw new Error("当前画面无法导出，请先打开本地视频文件。");
      const chosen = await plugin.dialogs.saveFile({ title: "导出片段", fileName: frameName(seen.source, range.start, ".mp4") });
      if (!chosen) return void (ui.clipStatus = "未选择保存位置。");
      await exporter.start(spec, seen, /\.mp4$/i.test(chosen) ? chosen : `${chosen}.mp4`);
      ui.clipStatus = "已开始导出，可在导出任务中查看。";
    } catch (error) {
      ui.clipStatus = message(error);
    } finally {
      ui.clipBusy = false;
    }
  });

  // ---------------------------------------------------------------- 导出任务
  list.bind(() => ({ taskRows: tasks.rows(), status: ui.tasksStatus }));
  const onTask = (name: string, action: (task: Task) => unknown) =>
    list.on(name, async (id: string) => {
      const task = tasks.find(String(id));
      if (!task) return;
      try {
        ui.tasksStatus = "";
        await action(task);
      } catch (error) {
        ui.tasksStatus = message(error);
      }
    });
  onTask("cancelTask", (task) => tasks.cancel(task));
  onTask("openOutput", (task) => tasks.open(task));
  onTask("retryTask", async (task) => {
    if (!terminal(task) || task.status === "completed") return;
    await exporter.retry(task);
    ui.tasksStatus = "已重新开始。";
  });
  list.on("openFolder", () => openFolder().catch((error) => (ui.tasksStatus = message(error))));
  plugin.onError((error) => {
    console.error(`[capture] ${error.message}`);
    ui.tasksStatus = message(error);
  });

  // ---------------------------------------------------------------- windows and menu
  async function show(window: Window, first?: () => Promise<void>) {
    try {
      if (!window.opened) await first?.();
      await window.show();
      if (errorWindow.opened) await errorWindow.close();
    } catch (error) {
      console.error(`[capture] window failed to open: ${message(error)}`);
      try {
        errorWindow.set({ message: "截图窗口无法打开，请关闭此提示后重试。" });
        await errorWindow.show();
      } catch (feedback) {
        console.error(`[capture] error window also failed: ${message(feedback)}`);
        say("截图窗口无法打开。");
      }
    }
  }
  // A fresh dialog starts its times at the current position.
  const startAtNow = (window: Window, withEnd: boolean) => async () => {
    const { position, duration } = await player.refresh();
    window.set({
      startTime: String(position ?? 0),
      ...(withEnd ? { endTime: String(Math.min((position ?? 0) + 10, duration ?? Infinity)), audioIndex: 1 } : {}),
    });
  };
  const canCapture = { enabled: () => usable(player.state), reason: () => (usable(player.state) ? null : "没有可截取的画面") };
  const entry = (id: string, title: string, order: number, run: () => unknown, extra: object = {}) =>
    plugin.commands.add({ id, path: ["视频", "截图", title], order, run, ...extra });
  entry("capture.save", "保存当前画面", 420, saveFrame, { bindings: ["Ctrl+E"], ...canCapture });
  entry("capture.saveAs", "当前画面另存为…", 421, saveFrameAs, canCapture);
  entry("capture.folder", "截图文件夹…", 422, chooseFolder);
  entry("capture.burst", "连拍…", 423, () => show(burst, startAtNow(burst, false)), { bindings: ["Ctrl+G"], group: "capture-export" });
  entry("capture.clip", "导出片段…", 424, () => show(clip, startAtNow(clip, true)), { group: "capture-export" });
  entry("capture.tasks", "导出任务…", 425, () => show(list), { group: "capture-export" });

  // Closing a window keeps tasks running; deactivation cancels them (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    exporter.stop();
    await tasks.stop();
  });
}
