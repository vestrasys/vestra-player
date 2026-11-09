import { access, constants, mkdir, stat } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";
import type { CommandOptions, Plugin, Text } from "@vestra-player/sdk";
import { clipboard } from "./copy.ts";
import { capturer } from "./form.ts";
import { extensionOf, Failure, FORMAT_LABEL, FORMATS, formatOfName, isLocal, message, problemOf, quality, REASON, usable } from "./model.ts";
import type { Format, Kind, Problem, Spec } from "./model.ts";
import { DEFAULT_TEMPLATES, insertVariable, normalTemplate, renderName } from "./names.ts";
import type { TemplateKind } from "./names.ts";
import { burstCount, burstStep, openSettings } from "./settings.ts";
import { Tasks } from "./tasks.ts";
import type { Task } from "./tasks.ts";

/**
 * An OSD line is at most 200 characters as shown: a path argument leaves room
 * for the words of the template (in any language) and falls back to the file
 * name when long; other arguments are cut.
 */
const ROOM = 160;
const fit = (path: string) => (path.length <= ROOM ? path : basename(path).slice(0, ROOM));
const cut = (text: string) => text.slice(0, ROOM);

/** The titles of the commands, by id. The first four are the right-click menu's Snapshot ▸. */
const TITLE = {
  "capture.save": "Save Current Frame",
  "capture.copy": "Copy Current Frame",
  "capture.burst": "Burst",
  "capture.settings": "Snapshot Settings…",
  "capture.saveAs": "Save Current Frame As…",
  "capture.saveScreen": "Save Rendered Frame",
  "capture.saveScreenAs": "Save Rendered Frame As…",
  "capture.copyScreen": "Copy Rendered Frame",
  "capture.format.png": "Capture as PNG",
  "capture.format.jpeg": "Capture as JPEG",
  "capture.format.bmp": "Capture as BMP",
  "capture.screenSubtitles": "Include Subtitles in Rendered Frame",
  "capture.folder": "Snapshot Folder…",
  "capture.openFolder": "Open Snapshot Folder",
  "capture.revealLast": "Show Last Capture in Folder",
  "capture.copyLast": "Copy Last Snapshot",
  "capture.saveVideo": "Save Video Frame Only",
  "capture.copyVideo": "Copy Video Frame Only",
} as const;
type CommandId = keyof typeof TITLE;
/** The commands shown in the right-click menu's Snapshot ▸; the rest are in the command palette only. */
const IN_MENU: readonly CommandId[] = ["capture.save", "capture.copy", "capture.burst", "capture.settings"];
/** The OSD prefix of a failed capture; the reason (or the player's or system's own message) goes after it. */
const FAILED = { image: "Snapshot failed: {}", burst: "Burst failed: {}" } as const;
/** The Options page (Options ▸ Video ▸ Snapshot). */
const PAGE_ID = "snapshot";

/** A button of a finished capture's message: a label and one of the plugin's own commands. */
interface OsdAction {
  /** Translated by the player like the text: a `plugin.i18n.text` value, so that extract finds it. */
  title: Text;
  command: "capture.revealLast" | "capture.copyLast";
}

/** The newest finished capture: what Show Last Capture in Folder and Copy Last Snapshot act on (this session). */
interface Saved {
  kind: Kind;
  /** The file (frame) or folder (burst). */
  path: string;
  /** A burst's first frame (Copy, Show in Folder). */
  first: string;
}

/**
 * Snapshot: save, save as or copy the current frame (the subtitles shown, or
 * the rendered frame with the video effects) as PNG, JPEG or BMP, and capture
 * bursts. No panel: the right-click menu's Snapshot ▸, the command palette
 * and the keys run it; its settings are a page of the Options window
 * (Options ▸ Video ▸ Snapshot). Exporting clips is a store plugin's job.
 * Every text is English source text (locales/ has the translations).
 */
export default async function activate(plugin: Plugin) {
  const { player, osd } = plugin;
  const { text } = plugin.i18n;
  const { settings, save: store } = openSettings(plugin);
  const ui = { stillBusy: false, burstBusy: false };
  // Headless hosts refuse the display; that never fails the action itself.
  const shown = (showing: Promise<void>) => void showing.catch(() => {});
  /** A reason on its own, or the player's or system's own message as it is. */
  function sayProblem({ reason, raw }: Problem) {
    if (reason) return shown(osd.show(REASON[reason]));
    // i18n-ignore: the player's or system's own message
    shown(osd.show(raw.slice(0, 200)));
  }
  /** "Snapshot failed: …", "Burst failed: …" with the reason, or the player's or system's own message. */
  const sayFailed = (kind: Kind, { reason, raw }: Problem) => shown(osd.show(FAILED[kind], [reason ? text(REASON[reason]) : cut(raw)]));
  // A failure while reporting a finished task goes to the log (it would otherwise be lost).
  const tasks = new Tasks(plugin, (task) => void finished(task).catch((error) => console.error(`[capture] finishing task ${task.id}: ${message(error)}`)));
  const exporter = capturer(plugin, tasks);
  const copier = clipboard(plugin);
  let sequence = 0;
  const nextNumber = () => ++sequence;
  let newest: Saved | null = null;

  /** Show a finished capture's message with its buttons (at most two), which act on the newest capture. */
  function showDone(message: "Frame saved as {}" | "Frame saved without subtitles as {}" | "Burst of {0} frames saved to {1}", args: (string | number)[], actions: OsdAction[]) {
    // A refusal is logged: a "saved" message that does not show is otherwise invisible (GUI check 2026-11-08).
    void osd.show(message, args, { actions: actions.slice(0, 2) }).catch((error: Error) => console.error(`[capture] osd: ${error.message}`));
  }

  /** A task reached its end: report it on screen; a finished one is the newest capture. */
  async function finished(task: Task) {
    const { kind } = task.spec;
    if (task.status === "completed") {
      const path = task.directory ?? task.outputs[0]?.path ?? task.destination;
      newest = { kind, path, first: kind === "burst" ? (task.outputs[0]?.path ?? "") : "" };
      const reveal: OsdAction = { title: text("Show in Folder"), command: "capture.revealLast" };
      const actions: OsdAction[] = kind === "image" ? [reveal, { title: text("Copy"), command: "capture.copyLast" }] : [reveal];
      // TRANSLATORS: {} is the file. The subtitles shown could not be drawn into the frame (only plain text can be).
      if (kind === "image" && task.spec.withoutSubtitles) showDone("Frame saved without subtitles as {}", [fit(path)], actions);
      else if (kind === "image") showDone("Frame saved as {}", [fit(path)], actions);
      // TRANSLATORS: {0} is the number of frames (2–20), {1} the folder.
      else showDone("Burst of {0} frames saved to {1}", [task.outputs.length, fit(task.destination)], actions);
    } else if (task.status === "failed") sayFailed(kind, task.error ?? { reason: "", raw: "" });
    await tasks.forget(task);
  }

  // ---------------------------------------------------------------- folders
  /** The folder captures go to now: the media's folder when asked and writable, else the capture folder. */
  async function targetFolder(source: string | null) {
    const beside = settings.besideMedia && isLocal(source) ? dirname(source) : null;
    if (beside && (await access(beside, constants.W_OK).then(() => true, () => false))) return beside;
    await mkdir(settings.folder, { recursive: true }).catch(() => {
      throw new Failure("folderUnavailable");
    });
    return settings.folder;
  }
  function setFormat(format: Format, say = true) {
    if (!FORMATS.includes(format)) return;
    if (settings.format !== format) {
      settings.format = format;
      store();
    }
    if (say) shown(osd.show("Snapshot format: {}", [FORMAT_LABEL[format]]));
  }
  function setQuality(value: unknown) {
    const next = quality(value);
    if (next === settings.quality) return;
    settings.quality = next;
    store();
  }
  type Flag = "screenSubtitles" | "besideMedia" | "snapshotEffects";
  const setFlag = (field: Flag, value: boolean) => {
    if (settings[field] === value) return;
    settings[field] = value;
    store();
  };
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
    const folder = await targetFolder((await player.refresh()).source);
    await plugin.system.openPath(folder).catch(() => {
      throw new Failure("cannotOpen");
    });
  };
  const openFolderSaying = () => openFolder().catch((error) => sayProblem(problemOf(error)));

  // ---------------------------------------------------------------- single frames
  /**
   * What a frame holds (user ruling 2026-11-07, as mpv's screenshot modes):
   * - `frame` (Save / Copy Current Frame; mpv `subtitles`): the source's size
   *   with the subtitles shown, and the video effects when Settings says so;
   * - `video` (Save / Copy Video Frame Only; mpv `video`): the source frame alone;
   * - `screen` (Save / Copy Rendered Frame): the video effects and, as set,
   *   the subtitles.
   */
  type FrameMode = "frame" | "video" | "screen";
  /** Whether the frame takes the effects (`processed`) and the subtitles. Subtitles come with `processed` (the player draws them into the processed frame). */
  async function frameContent(mode: FrameMode) {
    if (mode === "video") return { processed: false, subtitles: false };
    if (mode === "screen") return { processed: true, subtitles: settings.screenSubtitles };
    const shownSubtitles = await plugin.subtitles.refresh().then((state) => state.selected !== null, () => false);
    return { processed: settings.snapshotEffects || shownSubtitles, subtitles: shownSubtitles };
  }
  type Content = { processed: boolean; subtitles: boolean; withoutSubtitles?: boolean };
  /**
   * The frame at the position read when the command ran (mpv `screenshot`,
   * PotPlayer), sent as the capture time; the host's current frame only when
   * no position is known.
   */
  const frameSpec = (seen: { position: number | null }, content: Content, format: Format): Spec => ({
    kind: "image", start: seen.position ?? 0, count: 1, step: 1, processed: content.processed, subtitles: content.processed && content.subtitles,
    current: seen.position === null, format, quality: settings.quality, withoutSubtitles: content.withoutSubtitles,
  });
  /**
   * Run a capture with the mode's content. A frame that has the subtitles only
   * because they are shown, and whose subtitles the player cannot draw into a
   * frame (only plain text can be), is taken without them rather than failing.
   * Returns whether the subtitles were left out (the message then says so).
   */
  async function withContent(mode: FrameMode, run: (content: Content) => Promise<void>) {
    const content = await frameContent(mode);
    try {
      await run(content);
      return false;
    } catch (error) {
      const onlySubtitles = mode === "frame" && content.subtitles && !settings.snapshotEffects;
      if (!onlySubtitles || !(error instanceof Failure && error.code === "unavailable")) throw error;
      await run({ processed: false, subtitles: false, withoutSubtitles: true });
      return true;
    }
  }
  const imageName = (source: string | null, time: number, format: Format) =>
    `${renderName("image", settings.templates.image, { source, start: time, now: new Date(), n: nextNumber() })}${extensionOf(format)}`;
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
  /**
   * Ctrl+E / Ctrl+Alt+E: the frame shown at the press, into the capture
   * folder, no dialogs. The name states the time the host reports for the
   * frame it froze (mpv `screenshot-template` `%P`), so name and picture agree.
   */
  const saveFrame = (mode: FrameMode) =>
    single(async () => {
      const seen = await player.refresh();
      const folder = await targetFolder(seen.source);
      const format = settings.format;
      await withContent(mode, (content) => exporter.start(frameSpec(seen, content, format), seen, (time) => join(folder, imageName(seen.source, time, format))).then(() => {}));
      if (tasks.active().length > 1) shown(osd.show("Snapshot queued"));
    });
  /** The frame shown when the command ran, into a file the user picks; a typed extension picks the format once. */
  const saveFrameAs = (mode: FrameMode) =>
    single(async () => {
      const seen = await player.refresh();
      if (!usable(seen)) throw new Failure("unusable");
      const start = seen.position ?? 0;
      const chosen = await plugin.dialogs.saveFile({ title: mode === "screen" ? "Save Rendered Frame As" : "Save Current Frame As", fileName: imageName(seen.source, start, settings.format) });
      if (!chosen) return;
      const typed = formatOfName(chosen);
      const format = typed ?? settings.format;
      await withContent(mode, (content) => exporter.start(frameSpec(seen, content, format), seen, typed ? chosen : `${chosen}${extensionOf(format)}`).then(() => {}));
    });
  /** Ctrl+C / Ctrl+Alt+C (F03): always PNG, original resolution. */
  const copyFrame = (mode: FrameMode) =>
    single(async () => {
      const seen = await player.refresh();
      const dropped = await withContent(mode, (content) => copier.copy(seen, content.processed, content.subtitles));
      // The subtitles shown could not be drawn into the frame (only plain text can be): say so.
      shown(dropped ? osd.show("Frame copied without subtitles: they cannot be drawn into a frame") : osd.show("Frame copied to the clipboard"));
    });

  // ---------------------------------------------------------------- burst
  /** Ctrl+G: frames from the one shown at the press, with the settings' count, interval, effects and subtitles. */
  async function startBurst() {
    if (ui.burstBusy) return;
    ui.burstBusy = true; // exclusion before the first await
    try {
      const seen = await player.refresh();
      const { count, step, processed, subtitles } = settings.burst;
      // The frame at the press, sent as the capture time; the host's current frame when no position is known.
      const spec: Spec = {
        kind: "burst", start: seen.position ?? 0, count, step,
        processed, subtitles: processed && subtitles, current: seen.position === null,
        format: settings.format, quality: settings.quality,
      };
      const folder = await targetFolder(seen.source);
      await exporter.start(spec, seen, (time) =>
        join(folder, renderName("burst", settings.templates.burst, { source: seen.source, start: time, end: time + (spec.count - 1) * spec.step, now: new Date(), n: nextNumber() })),
      );
      if (tasks.active().length > 1) shown(osd.show("Snapshot queued"));
    } catch (error) {
      sayFailed("burst", problemOf(error));
    } finally {
      ui.burstBusy = false;
    }
  }

  // ---------------------------------------------------------------- settings page
  /**
   * What the page's text fields hold: the page owns the text while the user
   * types (a count of "1" on the way to "15", an empty template), the
   * settings keep the last valid value.
   */
  const typed = {
    count: String(settings.burst.count),
    step: String(settings.burst.step),
    image: settings.templates.image,
    burst: settings.templates.burst,
  };
  const page = plugin.settings.page("ui/settings.slint", { id: PAGE_ID, category: "video", title: "Snapshot", order: 50 });
  /**
   * The examples under the templates, from fixed sample values (file name,
   * position, date and time, number): the page reads no player state, and its
   * values stay the same from a park to a replay (lazy activation).
   */
  const SAMPLE = { source: "video.mp4", start: 309.412, end: 319.412, now: new Date(2026, 10, 7, 16, 21, 5), n: 1 };
  function previews() {
    const values = SAMPLE;
    return {
      image: `${renderName("image", settings.templates.image, values)}${extensionOf(settings.format)}`,
      burst: renderName("burst", settings.templates.burst, values),
    };
  }
  page.bind(() => ({
    folder: settings.folder,
    besideMedia: settings.besideMedia,
    formatIndex: FORMATS.indexOf(settings.format),
    quality: settings.quality,
    snapshotEffects: settings.snapshotEffects,
    burstCount: typed.count,
    burstStep: typed.step,
    burstProcessed: settings.burst.processed,
    burstSubtitles: settings.burst.subtitles,
    templateImage: typed.image,
    templateBurst: typed.burst,
    previews: previews(),
  }));
  const on = (name: string, run: (...args: never[]) => unknown) => page.on(name, run as (...args: unknown[]) => unknown);
  on("pickFolder", chooseFolder);
  on("setBesideMedia", (value: boolean) => setFlag("besideMedia", value === true));
  on("chooseFormat", (index: number) => setFormat(FORMATS[Number(index)]!, false));
  on("setQuality", (value: number) => setQuality(value));
  on("setSnapshotEffects", (value: boolean) => setFlag("snapshotEffects", value === true));
  on("burstEdited", (count: string, step: string) => {
    Object.assign(typed, { count: String(count), step: String(step) });
    const next = { count: burstCount(count) ?? settings.burst.count, step: burstStep(step) ?? settings.burst.step };
    if (next.count === settings.burst.count && next.step === settings.burst.step) return;
    Object.assign(settings.burst, next);
    store();
  });
  on("setBurstProcessed", (value: boolean) => {
    settings.burst.processed = value === true;
    store();
  });
  on("setBurstSubtitles", (value: boolean) => {
    settings.burst.subtitles = value === true;
    store();
  });
  const isKind = (kind: unknown): kind is TemplateKind => typeof kind === "string" && kind in DEFAULT_TEMPLATES;
  on("templateEdited", (kind: string, value: string) => {
    if (!isKind(kind)) return;
    typed[kind] = String(value);
    // An empty template is kept empty in the field while typing; names use the default meanwhile.
    settings.templates[kind] = normalTemplate(kind, value);
    store();
  });
  on("insertVariable", (kind: string, variable: string) => {
    if (!isKind(kind)) return;
    typed[kind] = settings.templates[kind] = insertVariable(settings.templates[kind], String(variable));
    store();
  });
  on("resetTemplate", (kind: string) => {
    if (!isKind(kind)) return;
    typed[kind] = settings.templates[kind] = DEFAULT_TEMPLATES[kind];
    store();
  });
  plugin.onError((error) => console.error(`[capture] ${error.message}`));

  // ---------------------------------------------------------------- commands
  /**
   * A frame to capture (`usable` in model.ts) as a condition the player
   * evaluates (`plugin.when`, player 0.5.21): the menu and the keys follow
   * playback without this plugin running. The player shows the reason only
   * while the condition is false.
   */
  const { when } = plugin;
  const canCapture = { enabled: when.can("snapshot").and(when.phase("ready", "ended"), when.stream.not()), reason: "No frame to capture" as const };
  /** The options besides id, path, order and run; `reason` and `category` are typed so that extract finds their texts. */
  type Extra = Omit<CommandOptions, "id" | "path" | "title" | "order" | "run" | "reason" | "category"> & { reason?: "No frame to capture"; category?: Text };
  /** Where an option's control lives: the settings page (Snapshot Settings…). */
  const inSettings = { home: "capture.settings" };
  // "Snapshot" is this plugin's own top-level submenu (menu sweep B5: moved out of the Video menu);
  // its first item's group "domain" puts it with Audio Track ▸ and Subtitle Track ▸, no separator between.
  // The command palette shows "Snapshot: Title" for every command.
  const entry = (id: CommandId, order: number, run: () => unknown, extra: Extra = {}) =>
    IN_MENU.includes(id)
      ? plugin.commands.add({ id, path: ["Snapshot", TITLE[id]], order, run, category: "Snapshot", ...extra })
      : plugin.commands.add({ id, title: TITLE[id], order, run, category: "Snapshot", ...extra });
  const quick = { group: "domain" };
  entry("capture.save", 420, () => saveFrame("frame"), { bindings: ["Ctrl+E"], ...quick, ...canCapture });
  entry("capture.copy", 422, () => copyFrame("frame"), { bindings: ["Ctrl+C"], ...quick, ...canCapture });
  entry("capture.burst", 426, startBurst, { bindings: ["Ctrl+G"], ...quick, ...canCapture });
  // Snapshot Settings…: the Options window at this plugin's page (as Playlist Settings… does).
  entry("capture.settings", 436, () =>
    plugin.settings.show(PAGE_ID).catch((error) => console.error(`[capture] the settings page could not open: ${message(error)}`)), { group: "capture-settings" });
  // Palette only (keys unchanged).
  entry("capture.saveAs", 421, () => saveFrameAs("frame"), canCapture);
  entry("capture.saveScreen", 423, () => saveFrame("screen"), { bindings: ["Ctrl+Alt+E"], ...canCapture });
  entry("capture.saveScreenAs", 424, () => saveFrameAs("screen"), canCapture);
  entry("capture.copyScreen", 425, () => copyFrame("screen"), { bindings: ["Ctrl+Alt+C"], ...canCapture });
  // The source frame alone (mpv's `screenshot video`), now that Save / Copy Current Frame take the subtitles shown.
  entry("capture.saveVideo", 446, () => saveFrame("video"), canCapture);
  entry("capture.copyVideo", 447, () => copyFrame("video"), canCapture);
  FORMATS.forEach((format, index) =>
    entry(`capture.format.${format}`, 430 + index, () => setFormat(format), { group: "capture-format", radio: true, checked: () => settings.format === format, ...inSettings }),
  );
  entry("capture.screenSubtitles", 435, () => setFlag("screenSubtitles", !settings.screenSubtitles), { checked: () => settings.screenSubtitles });
  entry("capture.folder", 437, chooseFolder, inSettings);
  entry("capture.openFolder", 438, openFolderSaying);
  /** The buttons of a finished capture's message, and palette commands: they act on the newest capture of this session. */
  const last = async () => {
    if (!newest) throw new Failure("noOutput");
    if (!(await stat(newest.path).catch(() => null))) throw new Failure("folderGone");
    return newest;
  };
  entry("capture.revealLast", 444, () =>
    last()
      .then((item) => plugin.system.reveal(item.kind === "burst" && item.first ? item.first : item.path))
      .catch((error) => sayProblem(problemOf(error))));
  entry("capture.copyLast", 445, () =>
    last()
      .then(async (item) => {
        const file = item.kind === "burst" ? item.first : item.path;
        // The clipboard takes PNG and JPEG.
        if (![".png", ".jpg", ".jpeg"].includes(extname(file).toLowerCase())) throw new Failure("clipboard");
        await plugin.clipboard.writeImage(file).catch(() => {
          throw new Failure("clipboard");
        });
        shown(osd.show("Frame copied to the clipboard"));
      })
      .catch((error) => sayProblem(problemOf(error))));

  // Deactivation cancels running captures (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    exporter.stop();
    await tasks.stop();
    await copier.stop();
  });
}
