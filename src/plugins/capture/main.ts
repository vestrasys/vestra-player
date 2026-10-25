import { access, constants, mkdir, stat } from "node:fs/promises";
import { basename, dirname, isAbsolute, join } from "node:path";
import type { CommandOptions, Plugin, Window } from "@vestra-player/sdk";
import { clipboard } from "./copy.ts";
import { capturer } from "./form.ts";
import {
  audioChoices, extensionOf, Failure, FORMAT_LABEL, FORMATS, formatOfName, frameName, message, parseRange, problemOf, quality, REASON, usable,
} from "./model.ts";
import type { Format, Kind, Problem, Spec } from "./model.ts";
import { openSettings } from "./settings.ts";
import { Tasks, terminal } from "./tasks.ts";
import type { Task } from "./tasks.ts";

/** A local file path (not a URL) whose folder can hold captures. */
const localFolder = (source: string | null) =>
  source && isAbsolute(source) && !/^[a-z][a-z0-9+.-]*:\/\//i.test(source) ? dirname(source) : null;
/**
 * An OSD line is at most 200 characters as shown: a path argument leaves room
 * for the words of the template (in any language) and falls back to the file
 * name when long; other arguments are cut.
 */
const ROOM = 160;
const fit = (path: string) => (path.length <= ROOM ? path : basename(path).slice(0, ROOM));
const cut = (text: string) => text.slice(0, ROOM);

/** What a dialog's status line says, as data; ui/parts.slint `Words.status` puts it into words. */
type Status = Problem<string> & { kind: string; count: number; name: string };
const status = (kind = "", fields: Partial<Status> = {}): Status => ({ kind, reason: "", raw: "", count: 0, name: "", ...fields });
const errorStatus = (error: unknown): Status => status("error", problemOf(error));

/** The menu titles under Video ▸ Snapshot, by command id. */
const MENU = {
  "capture.save": "Save Current Frame",
  "capture.saveAs": "Save Current Frame As…",
  "capture.copy": "Copy Current Frame",
  "capture.saveScreen": "Save Rendered Frame",
  "capture.saveScreenAs": "Save Rendered Frame As…",
  "capture.copyScreen": "Copy Rendered Frame",
  "capture.burst": "Burst…",
  "capture.clip": "Export Clip…",
  "capture.tasks": "Export Tasks…",
  "capture.format.png": "Capture as PNG",
  "capture.format.jpeg": "Capture as JPEG",
  "capture.format.bmp": "Capture as BMP",
  "capture.screenSubtitles": "Include Subtitles in Rendered Frame",
  "capture.settings": "Snapshot Settings…",
  "capture.folder": "Snapshot Folder…",
  "capture.openFolder": "Open Snapshot Folder",
} as const;
/** The OSD prefix of a failed capture; the reason (or the player's or system's own message) goes after it. */
const FAILED = { image: "Snapshot failed: {}", burst: "Burst failed: {}", clip: "Export failed: {}" } as const;

/**
 * Capture (decision D3): Save Current Frame saves straight into the capture
 * folder, Save As asks for a file, Burst / Export Clip / Export Tasks are
 * their own dialogs. Every text is English source text (locales/ has the
 * translations); the windows get data and say it in their Slint files.
 */
export default async function activate(plugin: Plugin) {
  const { player, osd } = plugin;
  const { text } = plugin.i18n;
  const { settings, save } = openSettings(plugin);
  const ui = { burstBusy: false, clipBusy: false, stillBusy: false, burstStatus: status(), clipStatus: status(), tasksStatus: status() };
  // Headless hosts refuse the display; that never fails the action itself.
  const shown = (showing: Promise<void>) => void showing.catch(() => {});
  /** A reason on its own, or the player's or system's own message as it is. */
  function sayProblem({ reason, raw }: Problem) {
    if (reason) return shown(osd.show(REASON[reason]));
    // i18n-ignore: the player's or system's own message
    shown(osd.show(raw.slice(0, 200)));
  }
  /** "Snapshot failed: …", "Burst failed: …", "Export failed: …" with the reason, or the player's or system's own message. */
  const sayFailed = (kind: Kind, { reason, raw }: Problem) => shown(osd.show(FAILED[kind], [reason ? text(REASON[reason]) : cut(raw)]));
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
    const { kind } = task.spec;
    if (task.status === "completed") {
      if (kind === "image") shown(osd.show("Frame saved as {}", [fit(path)]));
      else if (kind === "burst")
        shown(
          osd.show(
            // TRANSLATORS: {0} is the number of frames (2–20), {1} the folder.
            "Burst of {0} frames saved to {1}",
            [task.outputs.length, fit(task.destination)],
          ),
        );
      else shown(osd.show("Clip exported as {}", [fit(path)]));
      if (kind === "burst") ui.burstStatus = status("saved", { count: task.outputs.length });
      if (kind === "clip") ui.clipStatus = status("exported", { name: basename(path) });
    } else if (task.status === "failed") {
      const problem = task.error ?? { reason: "", raw: "" };
      sayFailed(kind, problem);
      if (kind === "burst") ui.burstStatus = status("burst-failed", problem);
      if (kind === "clip") ui.clipStatus = status("clip-failed", problem);
    }
  }

  // ---------------------------------------------------------------- settings
  const store = save;
  /** F07/F09: the playing file's folder when "Save to the media folder" is on and the source is local; the capture folder otherwise. */
  const shownFolder = (source: string | null) => (settings.besideMedia && localFolder(source)) || settings.folder;
  /** The folder captures go to now: falls back to the capture folder when the media folder cannot be written. */
  async function targetFolder(source: string | null) {
    const beside = settings.besideMedia ? localFolder(source) : null;
    if (beside && (await access(beside, constants.W_OK).then(() => true, () => false))) return beside;
    await mkdir(settings.folder, { recursive: true }).catch(() => {
      throw new Failure("folderUnavailable");
    });
    return settings.folder;
  }
  function setFormat(format: Format) {
    if (!FORMATS.includes(format)) return;
    if (settings.format !== format) {
      settings.format = format;
      store();
    }
    shown(osd.show("Snapshot format: {}", [FORMAT_LABEL[format]]));
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
  /** `screen`: the rendered frame (F04) — plugin effects and, when ticked, the subtitles. */
  const frameSpec = (seen: { position: number | null }, current: boolean, screen: boolean, format: Format): Spec => ({
    kind: "image", start: seen.position ?? 0, count: 1, step: 1, processed: screen, subtitles: screen && settings.screenSubtitles,
    track: "none", current, format, quality: settings.quality,
  });
  /** One single frame at a time (save, save as, copy); a second press is refused, not queued here. */
  async function single(action: () => Promise<void>) {
    if (ui.stillBusy) return shown(osd.show("The previous snapshot is still being processed"));
    ui.stillBusy = true;
    try {
      await action();
    } catch (error) {
      const problem = problemOf(error);
      if (problem.reason === "clipboard") sayProblem(problem);
      else sayFailed("image", problem);
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
      if (tasks.active().length > 1) shown(osd.show("Snapshot queued"));
    });
  /** The frame shown when the command ran, into a file the user picks; a typed extension picks the format once. */
  const saveFrameAs = (screen: boolean) =>
    single(async () => {
      const seen = await player.refresh();
      if (!usable(seen)) throw new Failure("unusable");
      const start = seen.position ?? 0;
      const chosen = await plugin.dialogs.saveFile({ title: screen ? "Save Rendered Frame As" : "Save Current Frame As", fileName: frameName(seen.source, start, extensionOf(settings.format)) });
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
      shown(osd.show("Frame copied to the clipboard"));
    });
  async function chooseFolder() {
    const chosen = await plugin.dialogs.openDirectory({ title: "Choose Snapshot Folder" });
    if (!chosen) return;
    if (!(await stat(chosen).catch(() => null))?.isDirectory()) return shown(osd.show("Please choose an existing folder."));
    settings.folder = chosen;
    store();
    shown(osd.show("Snapshot folder: {}", [fit(chosen)]));
  }
  /** F09: the folder captures go to now, created when missing, opened in the file manager. */
  const openFolder = async () => {
    const folder = await targetFolder(player.state.source);
    await plugin.system.openPath(folder).catch(() => {
      throw new Failure("cannotOpen");
    });
  };
  const openFolderSaying = () => openFolder().catch((error) => sayProblem(problemOf(error)));

  // ---------------------------------------------------------------- Snapshot Settings
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

  // ---------------------------------------------------------------- Burst
  burst.bind(() => {
    const last = tasks.latest("burst");
    return {
      folder: shownFolder(player.state.source),
      busy: ui.burstBusy,
      canCapture: usable(player.state),
      progress: tasks.progress(last),
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
      ui.burstStatus = status("started", { count: spec.count });
    } catch (error) {
      ui.burstStatus = errorStatus(error);
    } finally {
      ui.burstBusy = false;
    }
  });
  // ---------------------------------------------------------------- Export Clip
  clip.bind(() => ({
    busy: ui.clipBusy,
    canExport: usable(player.state),
    audioLabels: audioChoices(player.state, text).labels,
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
        track: audioChoices(seen, text).values[Number(audio)] ?? "none", current: false,
      };
      if (!usable(seen)) throw new Failure("unexportable");
      const chosen = await plugin.dialogs.saveFile({ title: "Export Clip", fileName: frameName(seen.source, range.start, ".mp4") });
      if (!chosen) return void (ui.clipStatus = status("no-destination"));
      await exporter.start(spec, seen, /\.mp4$/i.test(chosen) ? chosen : `${chosen}.mp4`);
      ui.clipStatus = status("clip-started");
    } catch (error) {
      ui.clipStatus = errorStatus(error);
    } finally {
      ui.clipBusy = false;
    }
  });

  // ---------------------------------------------------------------- Export Tasks
  list.bind(() => ({ taskRows: tasks.rows(), status: ui.tasksStatus }));
  const onTask = (name: string, action: (task: Task) => unknown) =>
    list.on(name, async (id: string) => {
      const task = tasks.find(String(id));
      if (!task) return;
      try {
        ui.tasksStatus = status();
        await action(task);
      } catch (error) {
        ui.tasksStatus = errorStatus(error);
      }
    });
  onTask("cancelTask", (task) => tasks.cancel(task));
  onTask("openOutput", (task) => tasks.open(task));
  onTask("retryTask", async (task) => {
    if (!terminal(task) || task.status === "completed") return;
    await exporter.retry(task);
    ui.tasksStatus = status("restarted");
  });
  list.on("openFolder", () => openFolder().catch((error) => (ui.tasksStatus = errorStatus(error))));
  plugin.onError((error) => {
    console.error(`[capture] ${error.message}`);
    ui.tasksStatus = errorStatus(error);
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
        await errorWindow.show(); // its text is in ui/error.slint
      } catch (feedback) {
        console.error(`[capture] error window also failed: ${message(feedback)}`);
        shown(osd.show("The snapshot window could not open."));
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
  const canCapture = { enabled: () => usable(player.state), reason: () => (usable(player.state) ? null : "No frame to capture" as const) };
  /** The options besides id, path, order and run; `reason` is typed so that extract finds its text. */
  type Extra = Omit<CommandOptions, "id" | "path" | "title" | "order" | "run" | "reason"> & { reason?: () => "No frame to capture" | null };
  // `@video` is the player's Video menu; "Snapshot" is this plugin's own submenu.
  const entry = (id: keyof typeof MENU, order: number, run: () => unknown, extra: Extra = {}) =>
    plugin.commands.add({ id, path: ["@video", "Snapshot", MENU[id]], order, run, ...extra });
  const source = { group: "capture-source", ...canCapture };
  const screen = { group: "capture-screen", ...canCapture };
  entry("capture.save", 420, () => saveFrame(false), { bindings: ["Ctrl+E"], ...source });
  entry("capture.saveAs", 421, () => saveFrameAs(false), source);
  entry("capture.copy", 422, () => copyFrame(false), { bindings: ["Ctrl+C"], ...source });
  entry("capture.saveScreen", 423, () => saveFrame(true), { bindings: ["Ctrl+Alt+E"], ...screen });
  entry("capture.saveScreenAs", 424, () => saveFrameAs(true), screen);
  entry("capture.copyScreen", 425, () => copyFrame(true), { bindings: ["Ctrl+Alt+C"], ...screen });
  // Checked while the dialog is open; closing it with the title bar (`onClose`) clears the mark at once.
  entry("capture.burst", 426, () => show(burst, startAtNow(burst, false)), {
    bindings: ["Ctrl+G"], group: "capture-export", checked: () => burst.opened,
  });
  burst.onClose(() => plugin.update());
  entry("capture.clip", 428, () => show(clip, startAtNow(clip, true)), { group: "capture-export" });
  entry("capture.tasks", 429, () => show(list), { group: "capture-export" });
  FORMATS.forEach((format, index) =>
    entry(`capture.format.${format}`, 430 + index, () => setFormat(format), {
      group: "capture-format", radio: true, checked: () => settings.format === format,
    }));
  entry("capture.screenSubtitles", 435, () => setFlag("screenSubtitles", !settings.screenSubtitles), {
    group: "capture-name", checked: () => settings.screenSubtitles,
  });
  entry("capture.settings", 436, () => show(settingsWindow), { group: "capture-name" });
  entry("capture.folder", 437, chooseFolder, { group: "capture-folder" });
  entry("capture.openFolder", 438, openFolderSaying, { group: "capture-folder" });

  // Closing a window keeps tasks running; deactivation cancels them (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    exporter.stop();
    await tasks.stop();
    await copier.stop();
  });
}
