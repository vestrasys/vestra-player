import { access, constants, mkdir, rm, stat } from "node:fs/promises";
import { basename, dirname, extname, join } from "node:path";
import type { CommandOptions, ImageRef, Panel, PanelState, Plugin, PlayerState, Text } from "@vestra-player/sdk";
import {
  abRange, AUDIO_RATES, defaultRange, effectiveMethod, estimate, fastContainer, HEIGHTS, keyframeArgs, codecArgs, parseCodec, LAST_SECONDS, lastRange, lastSeconds,
  METHODS, NO_TIMELINE, parseKeyframes, PRESET_ENCODE, presetOf, PRESETS, RATES, snapStart, SPEEDS, timeline, timelineWindow, crfOf,
} from "./clip.ts";
import type { Encode, Method } from "./clip.ts";
import { clipboard } from "./copy.ts";
import { capturer } from "./form.ts";
import {
  audioChoices, clippable, clock, extensionOf, Failure, FORMAT_LABEL, FORMATS, formatOfName, isLocal, message, parseRange, parseTime, problemOf, quality, REASON,
  tabMode, usable,
} from "./model.ts";
import type { Format, Kind, Problem, Spec } from "./model.ts";
import { DEFAULT_TEMPLATES, insertVariable, normalTemplate, renderName } from "./names.ts";
import type { TemplateKind } from "./names.ts";
import { makeThumb, Results, sizeOf } from "./results.ts";
import type { Saved } from "./results.ts";
import { hostSupport, rangeMarker, showDone } from "./seams.ts";
import type { HostSupport, OsdAction } from "./seams.ts";
import { openSettings } from "./settings.ts";
import { Tasks, terminal, timeLeft } from "./tasks.ts";
import type { Task } from "./tasks.ts";

/**
 * An OSD line is at most 200 characters as shown: a path argument leaves room
 * for the words of the template (in any language) and falls back to the file
 * name when long; other arguments are cut.
 */
const ROOM = 160;
const fit = (path: string) => (path.length <= ROOM ? path : basename(path).slice(0, ROOM));
const cut = (text: string) => text.slice(0, ROOM);

/** A status line of the tab, as data; ui/parts.slint `Words.status` puts it into words. */
type Status = Problem<string> & { kind: string; count: number; name: string };
const status = (kind = "", fields: Partial<Status> = {}): Status => ({ kind, reason: "", raw: "", count: 0, name: "", ...fields });
const errorStatus = (error: unknown): Status => status("error", problemOf(error));

/** The titles of the commands, by id. The first five are the right-click menu's Snapshot ▸ (proposal §6). */
const TITLE = {
  "capture.save": "Save Current Frame",
  "capture.copy": "Copy Current Frame",
  "capture.burst": "Burst…",
  "capture.clip": "Export Clip…",
  "capture.panel": "Capture…",
  "capture.saveAs": "Save Current Frame As…",
  "capture.saveScreen": "Save Rendered Frame",
  "capture.saveScreenAs": "Save Rendered Frame As…",
  "capture.copyScreen": "Copy Rendered Frame",
  "capture.tasks": "Export Tasks…",
  "capture.format.png": "Capture as PNG",
  "capture.format.jpeg": "Capture as JPEG",
  "capture.format.bmp": "Capture as BMP",
  "capture.screenSubtitles": "Include Subtitles in Rendered Frame",
  "capture.settings": "Snapshot Settings…",
  "capture.folder": "Snapshot Folder…",
  "capture.openFolder": "Open Snapshot Folder",
  "capture.clipStart": "Set Clip Start",
  "capture.clipEnd": "Set Clip End",
  "capture.clipExport": "Export Clip Range",
  "capture.revealLast": "Show Last Capture in Folder",
  "capture.copyLast": "Copy Last Snapshot",
  "capture.saveVideo": "Save Video Frame Only",
  "capture.copyVideo": "Copy Video Frame Only",
} as const;
type CommandId = keyof typeof TITLE;
/** The commands shown in the right-click menu's Snapshot ▸; the rest are in the command palette only. */
const IN_MENU: readonly CommandId[] = ["capture.save", "capture.copy", "capture.burst", "capture.clip", "capture.panel"];
/** The OSD prefix of a failed capture; the reason (or the player's or system's own message) goes after it. */
const FAILED = { image: "Snapshot failed: {}", burst: "Burst failed: {}", clip: "Export failed: {}" } as const;
/** The tab's pages, as its sub-tab strip orders them. */
const SECTIONS = ["snapshot", "clip", "results", "settings"] as const;
type Section = (typeof SECTIONS)[number];

/** What a field typed in the tab holds (the tab owns the text while it shows; the plugin keeps a copy). */
interface Form {
  start: string;
  end: string;
  /** The range the plugin itself last filled in (Export Clip…, Last N s): a range still equal to it was not typed by the user. */
  autoStart: string;
  autoEnd: string;
  audioIndex: number;
  processed: boolean;
  burn: boolean;
  burstStart: string;
  burstCount: string;
  burstStep: string;
}

export interface Options {
  /** Pretend the player offers more than it does (tests of the tab's views; seams.ts). */
  support: Partial<HostSupport>;
}

/** Megabytes as a number for the tab ("8.1", "71"); the tab adds the unit in the user's language. "" when unknown. */
const mb = (bytes: number) => (bytes <= 0 ? "" : bytes < 10e6 ? (bytes / 1e6).toFixed(1) : String(Math.round(bytes / 1e6)));
const minutesSince = (time: number) => Math.max(0, Math.floor((Date.now() - time) / 60_000));
const elapsed = (from: number | undefined) => {
  const seconds = from ? Math.max(0, Math.floor((Date.now() - from) / 1000)) : 0;
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
};

/**
 * The Capture tab (the Snapshots & Clips design of 2026-11-07): one sidebar tab beside Playlist
 * and Chapters with Snapshot, Clip, Results and Settings sections, replacing
 * the Burst, Export Clip, Export Tasks and Snapshot Settings windows. Every
 * command id stays; the right-click Snapshot ▸ keeps five items. Every text is
 * English source text (locales/ has the translations); the tab gets data and
 * says it in its Slint files.
 */
export default async function activate(plugin: Plugin, options: Partial<Options> = {}) {
  const { player, osd } = plugin;
  const { text } = plugin.i18n;
  const support = hostSupport(plugin, options.support);
  const { settings, save: store } = openSettings(plugin);
  const results = new Results(plugin);
  let thumbFolder: string | null = null;
  try {
    thumbFolder = join(plugin.dataDirectory, "thumbs");
  } catch {
    // no data directory: rows show their kind's icon only
  }
  const ui = { stillBusy: false, burstBusy: false, clipBusy: false, burstStatus: status(), clipStatus: status(), resultsStatus: status() };
  const form: Form = {
    start: "", end: "", autoStart: "", autoEnd: "", audioIndex: 1, processed: false, burn: false,
    burstStart: "0", burstCount: String(settings.burst.count), burstStep: String(settings.burst.step),
  };
  /** Finished results since the Results page was last on screen: the badge on its sub-tab. */
  let unseen = 0;
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
  // A failure while reporting a finished task goes to the log (it would otherwise be lost: no message, no result row).
  const tasks = new Tasks(plugin, (task) => void finished(task).catch((error) => console.error(`[capture] finishing task ${task.id}: ${message(error)}`)));
  const exporter = capturer(plugin, tasks);
  const copier = clipboard(plugin);
  const marker = rangeMarker(plugin, text("Clip"));
  let sequence = 0;
  const nextNumber = () => ++sequence;

  // ---------------------------------------------------------------- results
  const thumbs = new Map<string, ImageRef>();
  const thumbOf = (item: Saved) => {
    if (!item.thumb || !thumbFolder) return null;
    let ref = thumbs.get(item.id);
    if (!ref) thumbs.set(item.id, (ref = plugin.ui.image(join(thumbFolder, item.thumb), { maxSide: 128 })));
    return ref;
  };
  const dropThumb = (item: Saved) => {
    thumbs.delete(item.id);
    if (item.thumb && thumbFolder) void rm(join(thumbFolder, item.thumb), { force: true });
  };
  async function thumbnail(item: Saved) {
    const name = await makeThumb(plugin, thumbFolder, item);
    if (!name || !results.find(item.id)) return;
    results.update(item, { thumb: name });
    // Past ffmpeg's await: the row's picture changed outside SDK-invoked code.
    plugin.update();
  }
  void results.check().then((changed) => changed && plugin.update());

  /** A task reached its end: a finished one becomes a result (kept across restarts); report it on screen and in the tab. */
  async function finished(task: Task) {
    const { kind } = task.spec;
    const path = task.directory ?? task.outputs[0]?.path ?? task.destination;
    if (task.status === "completed") {
      const actual = task.actual ?? { start: task.spec.start, end: task.spec.end ?? task.spec.start };
      if (kind === "clip" && task.spec.method !== "fast" && task.started && task.ended && actual.end > actual.start)
        settings.preciseSpeed = Math.max(0.01, (task.ended - task.started) / 1000 / (actual.end - actual.start));
      if (kind === "clip") store();
      // R2: a clip's row states the range its file holds (a Fast cut's keyframe start).
      const { saved, dropped } = results.add({
        kind, path, first: kind === "burst" ? (task.outputs[0]?.path ?? "") : "", method: kind === "clip" ? (task.spec.method ?? "precise") : "",
        bytes: await sizeOf(path, kind), count: task.outputs.length, start: actual.start, end: actual.end, finished: task.ended ?? Date.now(), thumb: "",
      });
      dropped.forEach(dropThumb);
      // The Results sub-tab counts what finished while it was not in front (its badge), instead of pushing a line into the page shown.
      if (!(tabShown() && settings.page === 2)) unseen++;
      // R5: the buttons run commands that act on the newest result, which this now is.
      const actions: OsdAction[] = [
        { title: text("Show in Folder"), command: "capture.revealLast" },
        kind === "image" ? { title: text("Copy"), command: "capture.copyLast" } : { title: text("Show in Tab"), command: "capture.tasks" },
      ];
      // TRANSLATORS: {} is the file. The subtitles shown could not be drawn into the frame (only plain text can be).
      if (kind === "image" && task.spec.withoutSubtitles) showDone(plugin, "Frame saved without subtitles as {}", [fit(path)], actions);
      else if (kind === "image") showDone(plugin, "Frame saved as {}", [fit(path)], actions);
      // TRANSLATORS: {0} is the number of frames (2–20), {1} the folder.
      else if (kind === "burst") showDone(plugin, "Burst of {0} frames saved to {1}", [task.outputs.length, fit(task.destination)], actions);
      else showDone(plugin, "Clip exported as {}", [fit(path)], actions);
      if (kind === "burst") ui.burstStatus = status("saved", { count: task.outputs.length });
      if (kind === "clip") ui.clipStatus = status("exported", { name: basename(path) });
      await tasks.forget(task);
      plugin.update();
      void thumbnail(saved);
    } else if (task.status === "failed") {
      const problem = task.error ?? { reason: "", raw: "" };
      sayFailed(kind, problem);
      if (kind === "burst") ui.burstStatus = status("burst-failed", problem);
      if (kind === "clip") ui.clipStatus = status("clip-failed", problem);
    }
  }

  /** The Results rows: tasks still running, queued, failed or cancelled first (newest first), then the finished results. */
  function resultRows() {
    const taskRows = tasks.list
      .filter((task) => task.status !== "completed")
      .reverse()
      .map((task) => {
        const { spec } = task;
        const progress = tasks.progressOf(task);
        const share = task.progress && task.progress.total > 0 ? task.progress.done / task.progress.total : null;
        const left = timeLeft(task);
        return {
          id: `t${task.id}`,
          kind: spec.kind,
          name: basename(task.destination),
          state: task.status,
          reason: progress.reason,
          raw: progress.raw,
          done: progress.done,
          total: progress.total,
          method: spec.kind === "clip" ? (spec.method ?? "precise") : "",
          size: "",
          // A number for the tab (its words add "s" or "×"), or the format's name.
          badge: spec.kind === "clip" ? (spec.end! - spec.start).toFixed(1) : spec.kind === "burst" ? String(spec.count) : FORMAT_LABEL[spec.format ?? "png"],
          ago: -1,
          percent: share === null ? -1 : Math.round(share * 100),
          left,
          elapsed: elapsed(task.started),
          range: spec.kind === "clip" ? `${clock(spec.start)}–${clock(spec.end!)}` : clock(spec.start),
          thumb: null,
          cancellable: !terminal(task),
          retryable: terminal(task),
          openable: false,
          copyable: false,
          revealable: false,
        };
      });
    const savedRows = results.list.map((item) => {
      const gone = results.gone.has(item.id);
      const extension = extname(item.kind === "burst" ? item.first : item.path).toLowerCase();
      return {
        id: item.id,
        kind: item.kind,
        name: basename(item.path),
        state: gone ? "gone" : "completed",
        reason: "",
        raw: "",
        done: item.count,
        total: item.count,
        method: item.method,
        size: mb(item.bytes),
        badge: item.kind === "clip" ? (item.end - item.start).toFixed(1) : item.kind === "burst" ? String(item.count) : extension.slice(1).toUpperCase(),
        ago: minutesSince(item.finished),
        percent: -1,
        left: -1,
        elapsed: "",
        range: item.kind === "clip" ? `${clock(item.start)}–${clock(item.end)}` : clock(item.start),
        thumb: thumbOf(item),
        cancellable: false,
        retryable: false,
        openable: !gone,
        // The clipboard takes PNG and JPEG.
        copyable: !gone && item.kind !== "clip" && [".png", ".jpg", ".jpeg"].includes(extension),
        revealable: !gone,
      };
    });
    return [...taskRows, ...savedRows];
  }

  /** The progress card at the top of the tab: the running task and how many wait. */
  function card() {
    const task = tasks.running();
    if (!task) return { shown: false, id: "", kind: "", name: "", method: "", preset: "", percent: -1, left: -1, elapsed: "", queued: 0 };
    const share = task.progress && task.progress.total > 0 ? task.progress.done / task.progress.total : null;
    const { spec } = task;
    return {
      shown: true,
      id: task.id,
      kind: spec.kind,
      name: basename(task.destination),
      method: spec.kind === "clip" ? (spec.method ?? "precise") : "",
      preset: spec.kind === "clip" && spec.method !== "fast" && spec.encode ? presetOf(spec.encode) : spec.kind === "clip" && spec.method !== "fast" ? "high" : "",
      percent: share === null ? -1 : Math.round(share * 100),
      left: timeLeft(task),
      elapsed: elapsed(task.started),
      queued: tasks.queued(),
    };
  }

  // ---------------------------------------------------------------- folders
  /** F07/F09: the playing file's folder when "Save to the media folder" is on and the source is local; the capture folder otherwise. */
  const shownFolder = (source: string | null) => (settings.besideMedia && isLocal(source) ? dirname(source) : settings.folder);
  /** The folder captures go to now: falls back to the capture folder when the media folder cannot be written. */
  async function targetFolder(source: string | null) {
    const beside = settings.besideMedia && isLocal(source) ? dirname(source) : null;
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
  type Flag = "screenSubtitles" | "besideMedia" | "burstOpen" | "advancedOpen" | "snapshotEffects";
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
    const folder = await targetFolder(player.state.source);
    await plugin.system.openPath(folder).catch(() => {
      throw new Failure("cannotOpen");
    });
  };
  const openFolderSaying = () => openFolder().catch((error) => sayProblem(problemOf(error)));

  // ---------------------------------------------------------------- single frames
  /**
   * What a frame holds (user ruling 2026-11-07, as mpv's screenshot modes):
   * - `frame` (Save / Copy Current Frame, the tab's Save and Copy; mpv
   *   `subtitles`): the source's size with the subtitles shown, and the
   *   video effects when Settings says so;
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
  /**
   * The frame at the position read when the command ran (mpv `screenshot`,
   * PotPlayer), sent as the capture time; the host's current frame only when
   * no position is known.
   */
  const frameSpec = (seen: { position: number | null }, content: { processed: boolean; subtitles: boolean; withoutSubtitles?: boolean }, format: Format): Spec => ({
    kind: "image", start: seen.position ?? 0, count: 1, step: 1, processed: content.processed, subtitles: content.processed && content.subtitles,
    track: "none", current: seen.position === null, format, quality: settings.quality, withoutSubtitles: content.withoutSubtitles,
  });
  /**
   * Run a capture with the mode's content. A frame that has the subtitles only
   * because they are shown, and whose subtitles the player cannot draw into a
   * frame (only plain text can be), is taken without them rather than failing.
   */
  type Content = { processed: boolean; subtitles: boolean; withoutSubtitles?: boolean };
  /** Returns whether the subtitles were left out (the message then says so). */
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
  async function startBurst(start: string, count: string, step: string, fromNow: boolean, processed: boolean, subtitles: boolean) {
    if (ui.burstBusy) return;
    ui.burstBusy = true; // exclusion before the first await
    try {
      Object.assign(form, { burstStart: start, burstCount: count, burstStep: step });
      const from = fromNow ? null : parseRange(start).start;
      const seen = await player.refresh();
      // "From now": the frame at the press, sent as the capture time; the host's current frame when no position is known.
      const at = from ?? seen.position;
      const spec: Spec = {
        kind: "burst", start: at ?? 0, count: Number(count), step: Number(step),
        processed, subtitles: processed && subtitles, track: "none", current: at === null,
        format: settings.format, quality: settings.quality,
      };
      const folder = await targetFolder(seen.source);
      await exporter.start(spec, seen, (time) =>
        join(folder, renderName("burst", settings.templates.burst, { source: seen.source, start: time, end: time + (spec.count - 1) * spec.step, now: new Date(), n: nextNumber() })),
      );
      // Kept for the next run once accepted (the limits were checked).
      settings.burst = { count: spec.count, step: spec.step, fromNow, processed, subtitles };
      store();
      ui.burstStatus = status("started", { count: spec.count });
    } catch (error) {
      ui.burstStatus = errorStatus(error);
    } finally {
      ui.burstBusy = false;
    }
  }

  // ---------------------------------------------------------------- clip
  /** What the tab knows of the source for a Fast cut: keyframes around the range, the video codec (ffprobe). */
  const probe = { source: "", from: 0, to: 0, keyframes: [] as number[], video: null as string | null, offset: 0, timer: undefined as ReturnType<typeof setTimeout> | undefined, abort: null as AbortController | null };
  plugin.onDispose(() => {
    clearTimeout(probe.timer);
    probe.abort?.abort();
  });
  const method = (): Method => effectiveMethod(settings.method, support.fastCut);
  /** The range typed in the tab, or the reason it is none. */
  function typedRange(): { start: number; end: number } | { error: "time" | "endBeforeStart"; field: "start" | "end" } {
    const start = parseTime(form.start);
    const end = parseTime(form.end);
    if (start === null) return { error: "time", field: "start" };
    if (end === null) return { error: "time", field: "end" };
    if (end <= start) return { error: "endBeforeStart", field: "end" };
    return { start, end };
  }
  /** The audio track's codec (`AudioTrack.codec`), for the Fast container. */
  const audioCodec = (state: PlayerState) => {
    const value = audioChoices(state, text).values[form.audioIndex] ?? "none";
    if (value === "none") return null;
    const track = value === "selected" ? state.audioTracks?.find((t) => t.selected) : state.audioTracks?.find((t) => String(t.id) === value);
    return (track as { codec?: string } | undefined)?.codec ?? null;
  };
  /** Ask ffprobe for the keyframes around the range 200 ms after the typing stops (Fast only; no decoding). */
  function probeSoon() {
    clearTimeout(probe.timer);
    if (method() !== "fast") return;
    probe.timer = setTimeout(() => void probeNow(), 200);
    probe.timer.unref?.();
  }
  async function probeNow() {
    const state = player.state;
    const range = typedRange();
    if (!clippable(state) || "error" in range) return;
    const source = state.source!;
    const window = timelineWindow(range.start, range.end, state.duration);
    // Ten seconds more before the start: the keyframe a cut starts on can be that far back.
    const from = Math.max(0, window.from - 10);
    if (probe.source === source && probe.from <= from && probe.to >= window.to) return;
    probe.abort?.abort();
    const abort = (probe.abort = new AbortController());
    try {
      if (!(await plugin.tools.available("ffprobe"))) return;
      if (probe.source !== source) {
        Object.assign(probe, { video: null, offset: 0 });
        const codec = await plugin.tools.run("ffprobe", codecArgs(source), { timeoutMs: 5000, maxOutput: 4096, signal: abort.signal });
        Object.assign(probe, parseCodec(Buffer.from(codec.stdout).toString("utf8")));
      }
      // ffprobe reads container timestamps: the window moves by the file's start time, the keyframes come back by it.
      const result = await plugin.tools.run("ffprobe", keyframeArgs(source, from + probe.offset, window.to + probe.offset), { timeoutMs: 10_000, maxOutput: 4 << 20, signal: abort.signal });
      Object.assign(probe, { source, from, to: window.to, keyframes: parseKeyframes(Buffer.from(result.stdout).toString("utf8"), probe.offset) });
      // Past ffprobe's await: the note and the small timeline changed outside SDK-invoked code.
      plugin.update();
    } catch (error) {
      if (!abort.signal.aborted) console.error(`[capture] keyframes: ${message(error)}`);
    }
  }
  /** The clip section's derived values: field errors, the note, the small timeline, the estimate, the file name. */
  function clipView(state: PlayerState) {
    const range = typedRange();
    const chosen = method();
    const empty = { error: "", field: "", note: { method: chosen, start: "", end: "", early: "", snapped: false, container: "" }, timeline: NO_TIMELINE, estimate: { length: "", size: "", timelow: 0, timehigh: 0 }, name: "", span: null as { start: number; end: number } | null };
    // Nothing typed yet: no complaint, the fields show their placeholders.
    if (!clippable(state) || (!form.start.trim() && !form.end.trim())) return empty;
    if ("error" in range) return { ...empty, error: range.error, field: range.field };
    const sameSource = probe.source === state.source;
    const keyframes = sameSource ? probe.keyframes : [];
    const snapped = chosen === "fast" ? snapStart(keyframes, range.start) : null;
    const start = snapped ?? range.start;
    const container = chosen === "fast" ? fastContainer(sameSource ? probe.video : null, audioCodec(state)) : "mp4";
    const length = range.end - start;
    const fileBytes = isLocal(state.source) ? fileSize(state.source) : null;
    const guess = estimate({ method: chosen, length, fileBytes, duration: state.duration, encode: settings.encode, secondsPerSecond: settings.preciseSpeed, height: null });
    const sizeText = guess.sizehigh <= 0 ? "" : guess.sizelow === guess.sizehigh ? `${guess.sizelow}` : `${Math.round(guess.sizelow)}–${Math.round(guess.sizehigh)}`;
    const extension = container === "mkv" ? ".mkv" : ".mp4";
    return {
      error: "",
      field: "",
      note: { method: chosen, start: clock(start), end: clock(range.end), early: snapped !== null && range.start - snapped >= 0.0005 ? (range.start - snapped).toFixed(1) : "", snapped: snapped !== null, container },
      timeline: timeline(range, { start, end: range.end }, chosen === "fast" ? keyframes : [], state.position, state.duration, snapped),
      estimate: { length: length.toFixed(1), size: sizeText, timelow: guess.timelow, timehigh: guess.timehigh },
      name: `${renderName("clip", settings.templates.clip, { source: state.source, start, end: range.end, now: new Date(), n: sequence + 1 })}${extension}`,
      // What the file will hold: a Fast cut from its keyframe.
      span: { start, end: range.end },
    };
  }
  /** The playing file's size (the estimates), read once per source. */
  const sizes = new Map<string, number | null>();
  function fileSize(path: string) {
    if (!sizes.has(path)) {
      sizes.set(path, null);
      void stat(path).then(
        (info) => {
          sizes.set(path, info.size);
          plugin.update();
        },
        () => {},
      );
    }
    return sizes.get(path) ?? null;
  }

  /** Export a range: into the save folder, or (`ask`) where the user picks. */
  async function exportRange(range: { start: number; end: number }, ask: boolean) {
    const seen = await player.refresh();
    if (!clippable(seen)) throw new Failure(tabMode(seen) === "local" ? "needDuration" : tabMode(seen) === "network" ? "needLocal" : "unexportable");
    const chosen = method();
    const precise = chosen === "precise";
    const spec: Spec = {
      kind: "clip", start: range.start, end: range.end, count: 1, step: 1,
      processed: precise && form.processed, subtitles: precise && form.processed && form.burn,
      track: audioChoices(seen, text).values[form.audioIndex] ?? "none", current: false,
      method: chosen,
      // R6: the tab's encoder settings.
      encode: precise && support.encodeOptions ? { ...settings.encode } : undefined,
    };
    const snapped = chosen === "fast" && probe.source === seen.source ? snapStart(probe.keyframes, range.start) : null;
    const container = chosen === "fast" ? fastContainer(probe.source === seen.source ? probe.video : null, audioCodec(seen)) : "mp4";
    const name = `${renderName("clip", settings.templates.clip, { source: seen.source, start: snapped ?? range.start, end: range.end, now: new Date(), n: nextNumber() })}${container === "mkv" ? ".mkv" : ".mp4"}`;
    let destination: string;
    if (ask) {
      const picked = await plugin.dialogs.saveFile({ title: "Export Clip", fileName: name });
      if (!picked) return void (ui.clipStatus = status("no-destination"));
      destination = /\.(mp4|mkv)$/i.test(picked) ? picked : `${picked}${container === "mkv" ? ".mkv" : ".mp4"}`;
    } else destination = join(await targetFolder(seen.source), name);
    await exporter.start(spec, seen, destination);
    ui.clipStatus = status("clip-started");
  }
  /** The fields hold a range the user chose: not empty and not what the plugin last filled in. */
  const typedByUser = () =>
    !!(form.start.trim() || form.end.trim()) && !(form.start === form.autoStart && form.end === form.autoEnd);
  /** Export the typed range; `say`: a failure also goes on screen (a key, not the tab's button). */
  async function exportClip(start: string, end: string, ask: boolean, say = false) {
    if (ui.clipBusy) return;
    ui.clipBusy = true; // exclusion before the first await
    try {
      Object.assign(form, { start, end });
      await exportRange(parseRange(start, end) as { start: number; end: number }, ask);
    } catch (error) {
      ui.clipStatus = errorStatus(error);
      if (say) sayFailed("clip", problemOf(error));
    } finally {
      ui.clipBusy = false;
    }
  }
  /** "Last N s" (Ctrl+Shift+R): the seconds before the position, into the save folder, with the tab's method. */
  async function exportLast() {
    if (ui.clipBusy) return;
    ui.clipBusy = true;
    try {
      const seen = await player.refresh();
      if (!clippable(seen)) throw new Failure(tabMode(seen) === "network" ? "needLocal" : "unexportable");
      const range = lastRange(seen.position, settings.lastSeconds);
      // A range the user typed (or set with Now or A-B) stays; empty or filled in by the plugin, it shows what was exported.
      if (!typedByUser()) setRange(range, true);
      await exportRange(range, false);
      if (ui.clipStatus.kind === "clip-started") shown(osd.show("Exporting the last {} seconds", [settings.lastSeconds]));
    } catch (error) {
      ui.clipStatus = errorStatus(error);
      sayFailed("clip", problemOf(error));
    } finally {
      ui.clipBusy = false;
    }
  }
  /**
   * Put a range into the tab's fields: the user's own (Now, A-B), or one the
   * plugin fills in (`automatic`: Export Clip… on an untouched range, Last N s),
   * which a later automatic fill may replace.
   */
  function setRange(range: { start?: number; end?: number }, automatic = false) {
    if (range.start !== undefined) form.start = clock(range.start);
    if (range.end !== undefined) form.end = clock(range.end);
    if (automatic) Object.assign(form, { autoStart: form.start, autoEnd: form.end });
    panel.set({ clipStart: form.start, clipEnd: form.end });
    rememberForm();
    probeSoon();
  }
  const setMethod = (next: Method) => {
    if (!METHODS.includes(next) || settings.method === next) return;
    settings.method = next;
    store();
    probeSoon();
  };
  const setEncode = (next: Encode) => {
    settings.encode = next;
    store();
  };

  // ---------------------------------------------------------------- the tab
  const templateProps = () => ({ templateImage: settings.templates.image, templateClip: settings.templates.clip, templateBurst: settings.templates.burst });
  // A short title: the default sidebar width shows it whole beside Playlist and Chapters.
  const panel: Panel = plugin.panels.mount("sidebar", "ui/tab.slint", { title: "Capture" });
  const keyOf = (id: CommandId) => plugin.commands.keys(id)[0] ?? "";
  const lastKey = () => plugin.commands.keys("capture.last30")[0] ?? "";
  /** The source the examples use: the playing file while the tab shows (read then only), else a sample name. */
  let lastSource: string | null = null;
  const shownSource = () => lastSource ?? "video.mp4";
  /** Whether the tab was on screen at the last pass; a timer refreshes the rows meanwhile. */
  let live = false;
  let ticker: ReturnType<typeof setInterval> | undefined;
  plugin.onDispose(() => clearInterval(ticker));
  /**
   * The tab's values. What it shows of the player's state is read only while
   * the tab is on screen (lazy activation): a background tab reads no player
   * state, so nothing keeps the plugin running while idle (sidebar-tabs.md).
   * The rest reads no player state (cheap; no subscription).
   */
  panel.bind(() => {
    // A passive read (the player mirrors the layout to a running plugin; nothing is held): the tab came or went.
    const state = panel.state;
    const shown = !!state?.effective;
    if (shown !== live) queueMicrotask(() => follow(panel.state));
    const rows = resultRows();
    const view = shown ? liveProps() : {};
    return {
      ...view,
      keys: {
        save: keyOf("capture.save"), copy: keyOf("capture.copy"), savescreen: keyOf("capture.saveScreen"), copyscreen: keyOf("capture.copyScreen"),
        burst: keyOf("capture.burst"), clip: keyOf("capture.clipExport"), panel: keyOf("capture.panel"), last: lastKey(),
      },
      support: { fast: support.fastCut, encode: support.encodeOptions, progress: support.progress },
      formatIndex: FORMATS.indexOf(settings.format),
      quality: settings.quality,
      burstOpen: settings.burstOpen,
      burstFromNow: settings.burst.fromNow,
      burstProcessed: settings.burst.processed,
      burstSubtitles: settings.burst.subtitles,
      burstBusy: ui.burstBusy,
      burstStatus: ui.burstStatus,
      methodIndex: METHODS.indexOf(method()),
      audioIndex: form.audioIndex,
      processed: form.processed,
      burn: form.burn,
      presetIndex: (() => {
        const preset = presetOf(settings.encode);
        return preset === "custom" ? 3 : PRESETS.indexOf(preset);
      })(),
      encCrf: settings.encode.crf,
      encSource: settings.encode.rate === "source",
      encSpeed: SPEEDS.indexOf(settings.encode.speed),
      encHeight: HEIGHTS.indexOf(settings.encode.height),
      encFps: RATES.indexOf(settings.encode.fps),
      encAudio: AUDIO_RATES.indexOf(settings.encode.audio),
      advancedOpen: settings.advancedOpen,
      lastSeconds: settings.lastSeconds,
      lastIndex: (LAST_SECONDS as readonly number[]).indexOf(settings.lastSeconds),
      clipBusy: ui.clipBusy,
      queueFull: tasks.full(),
      clipStatus: ui.clipStatus,
      card: card(),
      results: rows,
      resultsStatus: ui.resultsStatus,
      folder: settings.folder,
      besideMedia: settings.besideMedia,
      previews: previews(),
      page: settings.page,
      unseen,
      snapshotEffects: settings.snapshotEffects,
    };
  });
  /** The examples under the templates: today's date and time, the playing file's name. */
  function previews() {
    const values = { source: shownSource(), start: 309.412, end: 319.412, now: new Date(), n: sequence + 1 };
    return {
      image: `${renderName("image", settings.templates.image, values)}${extensionOf(settings.format)}`,
      clip: `${renderName("clip", settings.templates.clip, { ...values, start: 242.3, end: 259 })}.mp4`,
      burst: renderName("burst", settings.templates.burst, values),
    };
  }

  /** The part of the tab that reads the player's state; read only while the tab shows. */
  function liveProps() {
    const state = player.state;
    lastSource = isLocal(state.source) ? state.source : null;
    const ab = abRange(state);
    const view = clipView(state);
    // R3: the clip's range on the seek bar while the tab shows (taken away when it goes, in `follow`).
    marker.set(view.span);
    return {
      mode: tabMode(state),
      canCapture: usable(state),
      canClip: clippable(state),
      audioLabels: audioChoices(state, text).labels,
      ab: { shown: !!ab, a: ab ? clock(ab.start) : "", b: ab ? clock(ab.end) : "" },
      canLast: clippable(state) && (state.position ?? 0) >= 0.1,
      folderShown: shownFolder(state.source),
      fieldError: view.error,
      errorField: view.field,
      note: view.note,
      timeline: view.timeline,
      estimate: view.estimate,
      clipName: view.name,
    };
  }
  /** The tab came on screen or went: what only matters while it shows starts or stops. */
  function follow(state: PanelState | null) {
    const on = !!state?.effective;
    if (on === live) return;
    live = on;
    clearInterval(ticker);
    if (on) {
      if (settings.page === 2) unseen = 0;
      // Rows say "1 min ago" and the card counts seconds: refresh them while the tab shows.
      ticker = setInterval(() => plugin.update(), 1000);
      ticker.unref?.();
      void results.check().then((changed) => changed && plugin.update());
      probeSoon();
    } else {
      marker.set(null);
      // What the user typed, written back for the next build of the tab (the player may rebuild it from the last values).
      panel.set({ clipStart: form.start, clipEnd: form.end, burstStart: form.burstStart, burstCount: form.burstCount, burstStep: form.burstStep });
    }
  }
  // Not `panel.onChange`: a listener of the layout would keep the plugin running while idle (lazy activation);
  // the binding above reads the state passively and is run again when it changes.
  void panel.ready.then(
    () => follow(panel.state),
    (error) => console.error(`[capture] the tab did not mount: ${message(error)}`),
  );
  /** Show the tab at a section; when it cannot open, the keys and Snapshot ▸'s direct items keep working. */
  /** The sub-tab shown, remembered; showing Results clears its badge. */
  function setPage(index: number) {
    if (!Number.isInteger(index) || index < 0 || index >= SECTIONS.length) return;
    if (index === 2) unseen = 0;
    if (settings.page === index) return;
    settings.page = index;
    store();
  }
  /** Show the tab at a page; when it cannot open, the keys and Snapshot ▸'s direct items keep working. */
  async function showTab(section: Section) {
    setPage(SECTIONS.indexOf(section));
    try {
      await panel.show();
    } catch (error) {
      console.error(`[capture] the tab could not open: ${message(error)}`);
      shown(osd.show("The Capture tab could not open."));
    }
  }
  const tabShown = () => !!panel.state?.effective;

  // The fields the tab owns are written by the plugin only on these actions.
  panel.set({ clipStart: form.start, clipEnd: form.end, burstStart: form.burstStart, burstCount: form.burstCount, burstStep: form.burstStep, ...templateProps() });
  /** The typed fields survive standby (session storage), as the tab's view must (sidebar-tabs.md). */
  let formTimer: ReturnType<typeof setTimeout> | undefined;
  function rememberForm() {
    clearTimeout(formTimer);
    formTimer = setTimeout(() => void plugin.storage.session.set("form", { ...form }).catch(() => {}), 500);
    formTimer.unref?.();
  }
  plugin.onDispose(() => clearTimeout(formTimer));
  void plugin.storage.session.get<Partial<Form>>("form").then(
    (kept) => {
      if (!kept || typeof kept !== "object") return;
      for (const key of ["start", "end", "autoStart", "autoEnd", "burstStart", "burstCount", "burstStep"] as const) if (typeof kept[key] === "string") form[key] = kept[key];
      if (Number.isInteger(kept.audioIndex)) form.audioIndex = kept.audioIndex!;
      for (const key of ["processed", "burn"] as const) if (typeof kept[key] === "boolean") form[key] = kept[key];
      panel.set({ clipStart: form.start, clipEnd: form.end, burstStart: form.burstStart, burstCount: form.burstCount, burstStep: form.burstStep });
      plugin.update();
    },
    () => {},
  );

  const on = (name: string, run: (...args: never[]) => unknown) => panel.on(name, run as (...args: unknown[]) => unknown);
  // Snapshot
  on("setPage", (index: number) => setPage(Number(index)));
  on("save", () => saveFrame("frame"));
  on("copy", () => copyFrame("frame"));
  on("setSnapshotEffects", (value: boolean) => setFlag("snapshotEffects", value === true));
  on("chooseFormat", (index: number) => setFormat(FORMATS[Number(index)]!));
  on("setQuality", (value: number) => setQuality(value));
  // Burst
  on("setBurstOpen", (open: boolean) => setFlag("burstOpen", open === true));
  on("burstNow", async () => {
    form.burstStart = clock((await player.refresh()).position ?? 0);
    panel.set({ burstStart: form.burstStart });
  });
  on("burstEdited", (start: string, count: string, step: string) => {
    Object.assign(form, { burstStart: String(start), burstCount: String(count), burstStep: String(step) });
    rememberForm();
  });
  on("startBurst", (start: string, count: string, step: string, fromNow: boolean, processed: boolean, subtitles: boolean) =>
    startBurst(String(start), String(count), String(step), fromNow === true, processed === true, subtitles === true));
  // Clip
  on("setMethod", (index: number) => setMethod(METHODS[Number(index)]!));
  on("clipEdited", (start: string, end: string) => {
    Object.assign(form, { start: String(start), end: String(end) });
    rememberForm();
    probeSoon();
  });
  on("clipNow", async (field: string) => {
    const position = (await player.refresh()).position ?? 0;
    setRange(field === "end" ? { end: position } : { start: position });
  });
  on("useAb", () => {
    const ab = abRange(player.state);
    if (!ab) return void (ui.clipStatus = status("error", { reason: "noLoop" }));
    setRange(ab);
  });
  on("last", () => exportLast());
  on("setAudio", (index: number) => {
    form.audioIndex = Math.max(0, Math.round(Number(index)));
    rememberForm();
  });
  on("setProcessed", (value: boolean) => {
    form.processed = value === true;
    rememberForm();
  });
  on("setBurn", (value: boolean) => {
    form.burn = value === true;
    rememberForm();
  });
  on("setPreset", (index: number) => {
    const preset = PRESETS[Number(index)];
    if (preset) setEncode({ ...PRESET_ENCODE[preset] });
  });
  on("setEncode", (field: string, value: number) => {
    const next = { ...settings.encode };
    const n = Math.round(Number(value));
    // Moving the quality number leaves "Same as source" for a quality level.
    if (field === "crf") Object.assign(next, { crf: crfOf(n), rate: "crf" });
    else if (field === "speed" && SPEEDS[n]) next.speed = SPEEDS[n]!;
    else if (field === "height" && HEIGHTS[n] !== undefined) next.height = HEIGHTS[n]!;
    else if (field === "fps" && RATES[n] !== undefined) next.fps = RATES[n]!;
    else if (field === "audio" && AUDIO_RATES[n] !== undefined) next.audio = AUDIO_RATES[n]!;
    else return;
    setEncode(next);
  });
  on("resetEncode", () => setEncode({ ...PRESET_ENCODE.high }));
  on("setAdvancedOpen", (open: boolean) => setFlag("advancedOpen", open === true));
  on("exportClip", (start: string, end: string) => exportClip(String(start), String(end), false));
  on("exportAs", (start: string, end: string) => exportClip(String(start), String(end), true));
  // Results
  const onRow = (name: string, action: (id: string) => unknown) =>
    on(name, async (id: string) => {
      try {
        ui.resultsStatus = status();
        await action(String(id));
      } catch (error) {
        ui.resultsStatus = errorStatus(error);
      }
    });
  const taskOf = (id: string) => (id.startsWith("t") ? tasks.find(id.slice(1)) : undefined);
  /** A result's file or folder, or the reason it cannot be used. */
  async function existing(id: string) {
    const item = results.find(id);
    if (!item) throw new Failure("noOutput");
    if (!(await stat(item.path).catch(() => null))) {
      results.gone.add(item.id);
      throw new Failure("folderGone");
    }
    return item;
  }
  onRow("cancelTask", async (id) => {
    const task = taskOf(id);
    if (task) await tasks.cancel(task);
  });
  onRow("retryTask", async (id) => {
    const task = taskOf(id);
    if (!task || !terminal(task) || task.status === "completed") return;
    await exporter.retry(task);
    await tasks.forget(task);
    ui.resultsStatus = status("restarted");
  });
  onRow("openResult", async (id) => {
    const item = await existing(id);
    await plugin.system.openPath(item.path).catch(() => {
      throw new Failure("cannotOpen");
    });
  });
  onRow("revealResult", async (id) => {
    const item = await existing(id);
    await plugin.system.reveal(item.kind === "burst" && item.first ? item.first : item.path);
  });
  onRow("copyResult", async (id) => {
    const item = await existing(id);
    await plugin.clipboard.writeImage(item.kind === "burst" ? item.first : item.path).catch(() => {
      throw new Failure("clipboard");
    });
    shown(osd.show("Frame copied to the clipboard"));
  });
  onRow("removeResult", async (id) => {
    const task = taskOf(id);
    if (task) return tasks.forget(task);
    const item = results.remove(id);
    if (item) dropThumb(item);
  });
  on("clearFinished", async () => {
    for (const task of tasks.list.filter(terminal)) await tasks.forget(task);
    results.clear().forEach(dropThumb);
  });
  on("openFolder", () => openFolder().catch((error) => (ui.resultsStatus = errorStatus(error))));
  // Settings
  on("pickFolder", chooseFolder);
  on("setBesideMedia", (value: boolean) => setFlag("besideMedia", value === true));
  on("templateEdited", (kind: string, value: string) => {
    if (!(kind in DEFAULT_TEMPLATES)) return;
    // An empty template is kept empty in the field while typing; names use the default meanwhile.
    settings.templates[kind as TemplateKind] = normalTemplate(kind as TemplateKind, value);
    store();
  });
  on("insertVariable", (kind: string, variable: string) => {
    if (!(kind in DEFAULT_TEMPLATES)) return;
    const next = insertVariable(settings.templates[kind as TemplateKind], String(variable));
    settings.templates[kind as TemplateKind] = next;
    store();
    panel.set({ ...templateProps() });
  });
  on("resetTemplate", (kind: string) => {
    if (!(kind in DEFAULT_TEMPLATES)) return;
    settings.templates[kind as TemplateKind] = DEFAULT_TEMPLATES[kind as TemplateKind];
    store();
    panel.set({ ...templateProps() });
  });
  on("setLastSeconds", (index: number) => {
    const next = lastSeconds(LAST_SECONDS[Number(index)]);
    if (next === settings.lastSeconds) return;
    settings.lastSeconds = next;
    store();
    void retitleLast();
  });
  on("focusChanged", (focused: boolean) => panel.setInteraction({ focused: focused === true }));
  plugin.onError((error) => {
    console.error(`[capture] ${error.message}`);
    ui.resultsStatus = errorStatus(error);
  });

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
  /**
   * The command palette shows "Category: Title" (player 0.7.6): "Snapshot" for
   * frames, bursts, the tab and the snapshot settings; "Export" for clips (a
   * clip is no snapshot).
   */
  const exporting = { category: text("Export") };
  /** Where an option's control lives now: the tab (proposal §6). */
  const inTab = { home: "capture.panel" };
  const inSettings = { home: "capture.settings" };
  // `@video` is the player's Video menu; "Snapshot" is this plugin's own submenu.
  const entry = (id: CommandId, order: number, run: () => unknown, extra: Extra = {}) =>
    IN_MENU.includes(id)
      ? plugin.commands.add({ id, path: ["@video", "Snapshot", TITLE[id]], order, run, category: "Snapshot", ...extra })
      : plugin.commands.add({ id, title: TITLE[id], order, run, category: "Snapshot", ...extra });
  const quick = { group: "capture-quick" };
  entry("capture.save", 420, () => saveFrame("frame"), { bindings: ["Ctrl+E"], ...quick, ...canCapture });
  entry("capture.copy", 422, () => copyFrame("frame"), { bindings: ["Ctrl+C"], ...quick, ...canCapture });
  // Ctrl+G: the tab's Burst row, open; checked while it shows.
  entry("capture.burst", 426, async () => {
    const fresh = !tabShown() || !settings.burstOpen;
    setFlag("burstOpen", true);
    if (fresh) {
      form.burstStart = clock((await player.refresh()).position ?? 0);
      panel.set({ burstStart: form.burstStart });
    }
    // Burst lives on the Settings page now (user ruling 2026-11-07); Ctrl+G shows it there, open.
    await showTab("settings");
  }, { bindings: ["Ctrl+G"], ...quick, checked: () => tabShown() && settings.page === 3 && settings.burstOpen });
  /**
   * Export Clip…: opens the tab at its Clip section (or scrolls there) and
   * never exports. A range the user has not typed is set to the position →
   * +10 s, as the dialog did.
   */
  async function openClip() {
    if (!typedByUser()) {
      const { position, duration } = await player.refresh();
      setRange(defaultRange(position, duration), true);
    }
    await showTab("clip");
  }
  entry("capture.clip", 428, openClip, { ...quick, ...exporting });
  /**
   * Export Clip Range (Ctrl+Shift+E, the Export button's key): exports the
   * tab's range into the snapshot folder; a failure is said on screen. With
   * no range yet it opens the Clip section instead.
   */
  entry("capture.clipExport", 427, async () => {
    if (!form.start.trim() && !form.end.trim()) return openClip();
    await exportClip(form.start, form.end, false, true);
  }, { bindings: ["Ctrl+Shift+E"], ...exporting, ...inTab, ...canCapture });
  // Alt+S: shows the tab, or hides the sidebar when the tab is in front (as F6 does for the playlist).
  entry("capture.panel", 440, async () => {
    try {
      await panel.toggle();
    } catch (error) {
      console.error(`[capture] the tab could not open: ${message(error)}`);
      shown(osd.show("The Capture tab could not open."));
    }
  }, { bindings: ["Alt+S"], group: "capture-tab", checked: () => (panel.state?.preferred ?? false) && (panel.state?.active ?? false) });
  // Palette only (keys unchanged).
  entry("capture.saveAs", 421, () => saveFrameAs("frame"), { ...inTab, ...canCapture });
  entry("capture.saveScreen", 423, () => saveFrame("screen"), { bindings: ["Ctrl+Alt+E"], ...inTab, ...canCapture });
  entry("capture.saveScreenAs", 424, () => saveFrameAs("screen"), { ...inTab, ...canCapture });
  entry("capture.copyScreen", 425, () => copyFrame("screen"), { bindings: ["Ctrl+Alt+C"], ...inTab, ...canCapture });
  // The source frame alone (mpv's `screenshot video`), now that Save / Copy Current Frame take the subtitles shown.
  entry("capture.saveVideo", 446, () => saveFrame("video"), { ...inTab, ...canCapture });
  entry("capture.copyVideo", 447, () => copyFrame("video"), { ...inTab, ...canCapture });
  entry("capture.tasks", 429, () => showTab("results"), exporting);
  FORMATS.forEach((format, index) =>
    entry(`capture.format.${format}`, 430 + index, () => setFormat(format), { group: "capture-format", radio: true, checked: () => settings.format === format, ...inSettings }),
  );
  entry("capture.screenSubtitles", 435, () => setFlag("screenSubtitles", !settings.screenSubtitles), { checked: () => settings.screenSubtitles, ...inTab });
  entry("capture.settings", 436, () => showTab("settings"));
  entry("capture.folder", 437, chooseFolder, inSettings);
  entry("capture.openFolder", 438, openFolderSaying);
  /**
   * The buttons of a finished capture's message (R5) and palette commands:
   * they act on the newest result. Players before 0.7.6 show no buttons, so
   * the same is in the tab's Results.
   */
  const newest = async () => {
    const item = results.list[0];
    if (!item) throw new Failure("noOutput");
    return existing(item.id);
  };
  entry("capture.revealLast", 444, () =>
    newest()
      .then((item) => plugin.system.reveal(item.kind === "burst" && item.first ? item.first : item.path))
      .catch((error) => sayProblem(problemOf(error))), { home: "capture.tasks" });
  entry("capture.copyLast", 445, () =>
    newest()
      .then(async (item) => {
        const file = item.kind === "burst" ? item.first : item.path;
        if (item.kind === "clip" || ![".png", ".jpg", ".jpeg"].includes(extname(file).toLowerCase())) throw new Failure("clipboard");
        await plugin.clipboard.writeImage(file).catch(() => {
          throw new Failure("clipboard");
        });
        shown(osd.show("Frame copied to the clipboard"));
      })
      .catch((error) => sayProblem(problemOf(error))), { home: "capture.tasks" });
  entry("capture.clipStart", 441, async () => {
    const position = (await player.refresh()).position ?? 0;
    setRange({ start: position });
    shown(osd.show("Clip start: {}", [clock(position)]));
  }, { ...exporting, ...inTab, ...canCapture });
  entry("capture.clipEnd", 442, async () => {
    const position = (await player.refresh()).position ?? 0;
    setRange({ end: position });
    shown(osd.show("Clip end: {}", [clock(position)]));
  }, { ...exporting, ...inTab, ...canCapture });
  /**
   * Export Last N Seconds (Ctrl+Shift+R): its title states the length the
   * user chose in Settings, so the command is declared again when it changes
   * (same id: the user's keys stay).
   */
  const addLast = () =>
    plugin.commands.add({
      id: "capture.last30",
      // TRANSLATORS: {} is 10, 30, 60 or 120.
      title: text("Export Last {} Seconds", [settings.lastSeconds]),
      category: text("Export"),
      order: 443,
      bindings: ["Ctrl+Shift+R"],
      home: "capture.panel",
      run: () => exportLast(),
      ...canCapture,
    });
  let lastCommand = addLast();
  async function retitleLast() {
    await lastCommand.dispose();
    lastCommand = addLast();
  }

  // Leaving the tab keeps tasks running; deactivation cancels them (the SDK releases jobs and snapshots).
  plugin.onDispose(async () => {
    exporter.stop();
    await tasks.stop();
    await copier.stop();
  });
}
