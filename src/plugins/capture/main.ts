import { access, constants, mkdir, stat } from "node:fs/promises";
import { basename, dirname, isAbsolute, join } from "node:path";
import type { Plugin, Window } from "@vestra-player/sdk";
import { ClipboardError, clipboard } from "./copy.ts";
import { capturer } from "./form.ts";
import { audioChoices, extensionOf, FORMAT_LABEL, FORMATS, formatOfName, frameName, message, parseRange, quality, usable } from "./model.ts";
import type { Format, Spec } from "./model.ts";
import { openSettings } from "./settings.ts";
import { Tasks, terminal } from "./tasks.ts";
import type { Task } from "./tasks.ts";

/** A local file path (not a URL) whose folder can hold captures. */
const localFolder = (source: string | null) =>
  source && isAbsolute(source) && !/^[a-z][a-z0-9+.-]*:\/\//i.test(source) ? dirname(source) : null;
/** An OSD line is at most 200 characters: fall back to the file name for long paths. */
const fit = (prefix: string, path: string) =>
  (prefix + path).length <= 200 ? prefix + path : (prefix + basename(path)).slice(0, 200);

/**
 * Capture (decision D3): 保存当前画面 saves straight into the capture folder,
 * 另存为 asks for a file, 连拍 / 导出片段 / 导出任务 are their own dialogs.
 */
export default async function activate(plugin: Plugin) {
  const { player, osd } = plugin;
  const { settings, save } = openSettings(plugin);
  const ui = { burstBusy: false, clipBusy: false, stillBusy: false, burstStatus: "", clipStatus: "", tasksStatus: "" };
  const say = (text: string) => void osd.show(text.slice(0, 200)).catch(() => {});
  const tasks = new Tasks(plugin, finished);
  const exporter = capturer(plugin, tasks);
  const copier = clipboard(plugin);
  const burst = plugin.windows.create("ui/burst.slint");
  const clip = plugin.windows.create("ui/clip.slint");
  const list = plugin.windows.create("ui/tasks.slint");
  const settingsWindow = plugin.windows.create("ui/settings.slint");
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

  // ---------------------------------------------------------------- settings
  const store = save;
  /** F07/F09: the playing file's folder when 储存到播放文件夹 is on and the source is local; the capture folder otherwise. */
  const shownFolder = (source: string | null) => (settings.besideMedia && localFolder(source)) || settings.folder;
  /** The folder captures go to now: falls back to the capture folder when the media folder cannot be written. */
  async function targetFolder(source: string | null) {
    const beside = settings.besideMedia ? localFolder(source) : null;
    if (beside && (await access(beside, constants.W_OK).then(() => true, () => false))) return beside;
    await mkdir(settings.folder, { recursive: true }).catch(() => {
      throw new Error("截图文件夹不可用，请重新选择。");
    });
    return settings.folder;
  }
  function setFormat(format: Format) {
    if (!FORMATS.includes(format)) return;
    if (settings.format !== format) {
      settings.format = format;
      store();
    }
    say(`截图格式：${FORMAT_LABEL[format]}`);
  }
  function setQuality(value: unknown) {
    const next = quality(value);
    if (next === settings.quality) return;
    settings.quality = next;
    store();
  }
  const setFlag = (field: "screenSubtitles" | "besideMedia", value: boolean) => {
    if (settings[field] === value) return;
    settings[field] = value;
    store();
  };

  // ---------------------------------------------------------------- single frames
  /** `screen`: 实际画面 (F04) — plugin effects and, when ticked, the subtitles. */
  const frameSpec = (seen: { position: number | null }, current: boolean, screen: boolean, format: Format): Spec => ({
    kind: "image", start: seen.position ?? 0, count: 1, step: 1, processed: screen, subtitles: screen && settings.screenSubtitles,
    track: "none", current, format, quality: settings.quality,
  });
  /** One single frame at a time (save, save as, copy); a second press is refused, not queued here. */
  async function single(action: () => Promise<void>) {
    if (ui.stillBusy) return say("上一张截图仍在处理");
    ui.stillBusy = true;
    try {
      await action();
    } catch (error) {
      say(error instanceof ClipboardError ? error.message : `截图失败：${message(error)}`);
    } finally {
      ui.stillBusy = false;
    }
  }
  /** Ctrl+E / Ctrl+Alt+E: the frame now, into the capture folder, no dialogs. */
  const saveFrame = (screen: boolean) =>
    single(async () => {
      const seen = await player.refresh();
      const folder = await targetFolder(seen.source);
      const format = settings.format;
      await exporter.start(frameSpec(seen, true, screen, format), seen, join(folder, frameName(seen.source, seen.position ?? 0, extensionOf(format))));
      if (tasks.active().length > 1) say("截图已排队");
    });
  /** The frame shown when the command ran, into a file the user picks; a typed extension picks the format once. */
  const saveFrameAs = (screen: boolean) =>
    single(async () => {
      const seen = await player.refresh();
      if (!usable(seen)) throw new Error("当前画面无法截取，请先打开本地视频文件。");
      const start = seen.position ?? 0;
      const title = screen ? "实际画面另存为" : "当前画面另存为";
      const chosen = await plugin.dialogs.saveFile({ title, fileName: frameName(seen.source, start, extensionOf(settings.format)) });
      if (!chosen) return;
      const typed = formatOfName(chosen);
      const format = typed ?? settings.format;
      await exporter.start(frameSpec(seen, false, screen, format), seen, typed ? chosen : `${chosen}${extensionOf(format)}`);
    });
  /** Ctrl+C / Ctrl+Alt+C (F03): always PNG, original resolution. */
  const copyFrame = (screen: boolean) =>
    single(async () => {
      const seen = await player.refresh();
      await copier.copy(seen, screen, settings.screenSubtitles);
      say("画面截图：复制到剪贴板");
    });
  async function chooseFolder() {
    const chosen = await plugin.dialogs.openDirectory({ title: "选择截图文件夹" });
    if (!chosen) return;
    if (!(await stat(chosen).catch(() => null))?.isDirectory()) return say("请选择一个已有的文件夹。");
    settings.folder = chosen;
    store();
    say(fit("截图文件夹：", chosen));
  }
  /** F09: the folder captures go to now, created when missing, opened in the file manager. */
  const openFolder = async () => {
    const folder = await targetFolder(player.state.source);
    await plugin.system.openPath(folder).catch(() => {
      throw new Error("无法打开文件夹。");
    });
  };
  const openFolderSaying = () => openFolder().catch((error) => say(message(error)));

  // ---------------------------------------------------------------- 截图设置
  settingsWindow.bind(() => ({
    formatIndex: FORMATS.indexOf(settings.format),
    quality: settings.quality,
    folder: settings.folder,
    besideMedia: settings.besideMedia,
    screenSubtitles: settings.screenSubtitles,
  }));
  settingsWindow.on("chooseFormat", (index: number) => setFormat(FORMATS[Number(index)]!));
  settingsWindow.on("setQuality", (value: number) => setQuality(value));
  settingsWindow.on("pickFolder", chooseFolder);
  settingsWindow.on("setBesideMedia", (value: boolean) => setFlag("besideMedia", value === true));
  settingsWindow.on("setScreenSubtitles", (value: boolean) => setFlag("screenSubtitles", value === true));

  // ---------------------------------------------------------------- 连拍
  burst.bind(() => {
    const last = tasks.latest("burst");
    return {
      folder: shownFolder(player.state.source),
      busy: ui.burstBusy,
      canCapture: usable(player.state),
      progress: last ? tasks.progress(last) : "",
      status: ui.burstStatus,
      formatIndex: FORMATS.indexOf(settings.format),
    };
  });
  burst.on("pickFolder", chooseFolder);
  burst.on("openFolder", openFolderSaying);
  burst.on("captureFrame", () => saveFrame(false));
  burst.on("chooseFormat", (index: number) => setFormat(FORMATS[Number(index)]!));
  burst.on("markStart", async () => burst.set({ startTime: String((await player.refresh()).position ?? 0) }));
  burst.on("startBurst", async (start: string, count: string, step: string, current: boolean, processed: boolean, subtitles: boolean) => {
    if (ui.burstBusy) return;
    ui.burstBusy = true; // exclusion before the first await
    try {
      const spec: Spec = {
        kind: "burst", start: current ? 0 : parseRange(start).start, count: Number(count), step: Number(step),
        processed: processed === true, subtitles: subtitles === true, track: "none", current: current === true,
        format: settings.format, quality: settings.quality,
      };
      const seen = await player.refresh();
      await exporter.start(spec, seen, await targetFolder(seen.source));
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
  const source = { group: "capture-source", ...canCapture };
  const screen = { group: "capture-screen", ...canCapture };
  entry("capture.save", "保存当前画面", 420, () => saveFrame(false), { bindings: ["Ctrl+E"], ...source });
  entry("capture.saveAs", "当前画面另存为…", 421, () => saveFrameAs(false), source);
  entry("capture.copy", "复制当前画面", 422, () => copyFrame(false), { bindings: ["Ctrl+C"], ...source });
  entry("capture.saveScreen", "保存实际画面", 423, () => saveFrame(true), { bindings: ["Ctrl+Alt+E"], ...screen });
  entry("capture.saveScreenAs", "实际画面另存为…", 424, () => saveFrameAs(true), screen);
  entry("capture.copyScreen", "复制实际画面", 425, () => copyFrame(true), { bindings: ["Ctrl+Alt+C"], ...screen });
  // Checked while the dialog is open; closing it with the title bar (`onClose`) clears the mark at once.
  entry("capture.burst", "连拍…", 426, () => show(burst, startAtNow(burst, false)), {
    bindings: ["Ctrl+G"], group: "capture-export", checked: () => burst.opened,
  });
  burst.onClose(() => plugin.update());
  entry("capture.clip", "导出片段…", 428, () => show(clip, startAtNow(clip, true)), { group: "capture-export" });
  entry("capture.tasks", "导出任务…", 429, () => show(list), { group: "capture-export" });
  FORMATS.forEach((format, index) =>
    entry(`capture.format.${format}`, `截取为 ${FORMAT_LABEL[format]}`, 430 + index, () => setFormat(format), {
      group: "capture-format", radio: true, checked: () => settings.format === format,
    }));
  entry("capture.screenSubtitles", "实际画面包含字幕", 435, () => setFlag("screenSubtitles", !settings.screenSubtitles), {
    group: "capture-name", checked: () => settings.screenSubtitles,
  });
  entry("capture.settings", "截图设置…", 436, () => show(settingsWindow), { group: "capture-name" });
  entry("capture.folder", "截图文件夹…", 437, chooseFolder, { group: "capture-folder" });
  entry("capture.openFolder", "打开截图文件夹", 438, openFolderSaying, { group: "capture-folder" });

  // Closing a window keeps tasks running; deactivation cancels them (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    exporter.stop();
    await tasks.stop();
    await copier.stop();
  });
}
