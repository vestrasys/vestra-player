import { createHash } from "node:crypto";
import { watch } from "node:fs";
import type { FSWatcher } from "node:fs";
import { stat } from "node:fs/promises";
import { basename, dirname } from "node:path";
import type {
  CommandOptions,
  Plugin,
  SubtitleStyle,
  SubtitleTrack,
  SubtitlesState,
  Text,
  TextArg,
} from "@vestra-player/sdk";
import { ENCODINGS, convert, cuesOf, parseSrt, readFileBytes } from "./formats.ts";
import type { Cue, Converted, Encoding } from "./formats.ts";
import { Failure, NONE, confirmed, raw, status as message, statusText } from "./messages.ts";
import type { FileError, Status, StatusId } from "./messages.ts";
import {
  DEFAULT_BOX,
  DEFAULT_SHADOW,
  SECONDARY_POSITIONS,
  SYNC_LIMIT,
  SYNC_MENU,
  SYNC_STEPS,
  clock,
  dialogueTarget,
  isObject,
  parse,
  parseStyle,
  parseSync,
  saveable,
  serialize,
} from "./model.ts";
import type { Jump, SavedStyle, SecondaryPosition, StyleFile } from "./model.ts";

/** Colour choices, in the order of ui/settings.slint's list. */
const COLORS = ["white", "yellow", "cyan"] as const;
/** Subtitles ▸ Subtitle Track lists at most this many tracks (orders 303–310); the rest are in Subtitle Tuning. */
const MENU_TRACKS = 8;
const SIZE_STEP = 2;
/** Menu keys the track items must not take: the fixed items of Subtitle Track ▸ (menus merge by these words). */
const RESERVED = new Set(["Off", "Cycle"]);
/** A style with every optional field filled in. */
type FullStyle = Required<SubtitleStyle>;
const BASE_FIELDS: Pick<FullStyle, "bold" | "shadow" | "box" | "assOverride"> = {
  bold: false, shadow: 0, box: null, assOverride: "keep",
};
const BASE = { fontSize: 36, color: "white", outline: 2, marginPercent: 5, offsetMs: 0, ...BASE_FIELDS } as const;
const NO_TEXT = "No text subtitles available";
const NO_SEEK = "The current media cannot seek";
const isRemote = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source) && !/^file:\/\//i.test(source);
const BUSY = "Subtitle operation in progress";
const NO_MEDIA = "No media to play";
const SECOND_TEXT_ONLY = "Only text subtitles can be secondary";
const SECOND_IS_PRIMARY = "Already the primary subtitle";

/** One short sentence for any failure; host details go to the log only. */
function errorText(e: unknown): Status {
  const error = e as { code?: string; name?: string; message?: string };
  if (e instanceof Failure) return e.status;
  if (error?.name === "AbortError") return message("cancelled");
  switch (error?.code) {
    case undefined:
      return raw(error?.message ?? String(e)); // not ours: the system's or the SDK's own words
    case "stale":
      return message("stale");
    case "invalid":
      return message("invalid");
    case "busy":
      return message("playerBusy");
    case "unavailable":
    case "unsupported":
      return message("unsupported");
    case "backend":
      return message("backend");
    default:
      return message("failed");
  }
}
/** Path segments: no control characters or edge spaces. */
const clean = (s: string) => s.replace(/[\u0000-\u001f\u007f]/g, " ").trim();
const seconds = (ms: number) => `${ms > 0 ? "+" : ms < 0 ? "−" : ""}${Math.abs(ms) / 1000}`;
/** "Cannot reload: …" for each error of reading a subtitle file. */
const RELOAD: Record<FileError, StatusId> = {
  readFailed: "reload.readFailed",
  notRegular: "reload.notRegular",
  emptyOrLarge: "reload.emptyOrLarge",
  bitmap: "reload.bitmap",
  unknownFormat: "reload.unknownFormat",
  noCues: "reload.noCues",
  tooLarge: "reload.tooLarge",
};
const clamp = (value: number, min: number, max: number) => Math.round(Math.max(min, Math.min(max, value)));

type Action = (ticket: number) => Promise<void>;
/**
 * A track this plugin loaded: where it came from, to reload it (A09), and its
 * cues for B11. `part` is its place among the tracks of the file (SAMI languages). Memory only.
 */
type Loaded = { path: string; part: number; hash: string; cues: Cue[] };
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** Subtitles: Subtitle Tuning and Subtitle Settings windows, Subtitles ▸ menu, serialized subtitle operations. */
export default async function activate(plugin: Plugin) {
  const { subtitles, osd } = plugin;
  const { text } = plugin.i18n;
  /** A track without a title: "Embedded 3" / "External subtitle", translated. */
  const fallbackName = (t: SubtitleTrack): Text =>
    t.kind === "embedded" ? text("Embedded {}", [t.embeddedId ?? ""]) : text("External subtitle");
  /** A track's name: its title and language (data), or the translated fallback name and the language. */
  const titled = (t: SubtitleTrack) => `${t.title}${t.language ? ` · ${t.language}` : ""}`;
  const label = (t: SubtitleTrack): TextArg =>
    t.title ? titled(t) : t.language ? text("{} · {}", [fallbackName(t), t.language]) : fallbackName(t);
  const tune = plugin.windows.create("ui/tune.slint", { id: "subtitle-tune" });
  const settings = plugin.windows.create("ui/settings.slint", { id: "subtitle-settings" });
  const syncWindow = plugin.windows.create("ui/sync.slint", { id: "subtitle-sync" });
  const settingsStore = store<Record<string, unknown>>(plugin, "settings", {});
  const stored = parse(settingsStore.value);
  let syncStep = stored.syncStepMs;
  let autoReload = stored.autoReload;
  let fallbackEncoding: Encoding = stored.fallbackEncoding;
  let secondaryPosition: SecondaryPosition = stored.secondaryPosition;
  const save = () =>
    settingsStore.save(serialize({ ...stored, syncStepMs: syncStep, autoReload, fallbackEncoding, secondaryPosition }));
  // style.json (D09): the style without the sync, restored once a stylable track is selected.
  const styleStore = store<Record<string, unknown>>(plugin, "style", {});
  const fontBase = (): SavedStyle => saveable({ fontId: subtitles.state?.fonts[0]?.id ?? "", ...BASE });
  let styleFile: StyleFile = parseStyle(styleStore.value, fontBase());
  let restorePending = styleFile.style !== null;
  const saveStyleFile = (next: Partial<StyleFile>) => {
    styleFile = { ...styleFile, ...next };
    styleStore.save({ version: 1, ...styleFile });
  };
  /** Tracks this plugin loaded on the current source, by track id. */
  const loaded = new Map<string, Loaded>();
  let status: Status = NONE;
  let busy = false;
  let epoch = 0; // bumped by cancel and media switches; stale actions stop at the next guard
  let aborter = new AbortController(); // aborted together with each epoch bump (stops an upload)
  let chain = Promise.resolve();
  let desired: SubtitleStyle | undefined; // edited style shown before the host confirms it
  let editing: SubtitlesState | null = null; // selection the pending style edit belongs to
  let styleTimer: ReturnType<typeof setTimeout> | undefined;
  let seen: SubtitlesState | null = null;
  let lastShown: string | null = null; // track id that Show Subtitles brings back
  // Headless hosts refuse the display; that never fails the action itself.
  const shown = (p: Promise<void>) => void p.catch(() => {});
  /** A status on screen, with its notes as further sentences ("A. B" / "A。B"). */
  // i18n-ignore: a text value, or the player's or the system's own message ("" clears)
  const say = (s: Status) => shown(osd.show(statusText(plugin.i18n, s)));
  /** The OSD wording for a subtitle offset (positive = later). */
  const sayOffset = (ms: number) =>
    shown(
      ms === 0
        ? osd.show("Subtitle sync: 0 s")
        : osd.show(ms < 0 ? "Subtitle sync: {} s earlier" : "Subtitle sync: {} s later", [Math.abs(ms) / 1000]),
    );

  const guard = (ticket: number, source?: number) => {
    if (epoch !== ticket || (source !== undefined && source !== subtitles.state?.sourceId))
      throw new Failure("cancelled");
  };
  /** One operation at a time; the menu shows busy meanwhile. Resolves to the final status. */
  function run(text: Status, action: Action) {
    const ticket = epoch;
    chain = chain.then(async () => {
      if (epoch !== ticket) return;
      busy = true;
      status = text;
      plugin.update();
      try {
        await action(ticket);
      } catch (e) {
        status = errorText(e);
        if ((e as { code?: string }).code) console.error(`[subtitles] ${(e as Error).message}`);
      } finally {
        busy = false;
        if (!subtitles.state) await subtitles.refresh().catch(() => {});
        plugin.update(); // also runs from the style timer, outside any SDK handler
      }
    });
    return chain.then(() => status);
  }
  /** A menu or key action: same operation, result shown as an on-screen message. */
  const announce = (text: Status, action: Action) => run(text, action).then(say);
  function stopStyleTimer() {
    clearTimeout(styleTimer);
    styleTimer = undefined;
    editing = null;
  }
  function cancel() {
    ++epoch;
    aborter.abort();
    aborter = new AbortController();
    desired = undefined;
    cycling = null;
    stopStyleTimer();
    status = message("cancelledByUser");
  }

  const refresh: Action = async () => {
    await subtitles.refresh();
    desired = undefined;
    status = message("refreshed");
  };
  /**
   * Primary selection. Choosing the track shown as the secondary moves it: the
   * secondary is turned off first (the player refuses the same track in both).
   */
  const select =
    (track: SubtitleTrack | null, source?: number): Action =>
    async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source ?? s.sourceId);
      let base = s;
      const moved = !!track && s.secondary?.id === track.id;
      if (moved) {
        base = await subtitles.select(null, { role: "secondary", ifCurrent: s });
        guard(ticket, s.sourceId);
      }
      await subtitles.select(track, { ifCurrent: base });
      guard(ticket, s.sourceId);
      desired = undefined;
      const state = subtitles.state?.trackConfirmation;
      status = !track ? message("hidden") : message(confirmed(moved ? "moved" : "select", state), [label(track)]);
    };
  /** Why `track` cannot be the secondary subtitle in `s`, or null. */
  const secondBlocked = (track: SubtitleTrack, s = subtitles.state) =>
    !track.text ? SECOND_TEXT_ONLY : track.id === s?.selected?.id ? SECOND_IS_PRIMARY : null;
  /** A08 secondary subtitle: a text track other than the primary, or off. */
  const selectSecondary =
    (track: SubtitleTrack | null, source?: number): Action =>
    async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source ?? s.sourceId);
      const now = track && s.tracks.find((t) => t.id === track.id);
      if (track && !now) throw new Failure("tracksChanged");
      const blocked = now && secondBlocked(now, s);
      if (blocked) throw new Failure(blocked === SECOND_TEXT_ONLY ? "secondTextOnly" : "secondIsPrimary");
      await subtitles.select(now ?? null, { role: "secondary", ifCurrent: s });
      guard(ticket, s.sourceId);
      status = now
        ? message(confirmed("secondary", subtitles.state?.trackConfirmation), [label(now)])
        : message("secondaryOff");
    };
  /** The video frame rate for MicroDVD files, when the player knows it. */
  const videoFps = () =>
    plugin.player
      .stats()
      .then((stats) => stats.video?.fps ?? null, () => null);
  /** Read and convert a subtitle file (D18). */
  const readSubtitle = async (path: string) => convert(await readFileBytes(path), { fallback: fallbackEncoding, fps: await videoFps() });
  const trackTitle = (path: string, part: Converted) =>
    `${basename(path)}${part.language ? ` · ${part.language}` : ""}`.replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 200);
  /**
   * Open Subtitle… (primary) or Add Secondary Subtitle… (A08: the first track of the file becomes the
   * secondary, the primary stays). Same reading and conversion either way (D18).
   */
  const loadAs = (role: "primary" | "secondary"): Action => async (ticket) => {
    const signal = aborter.signal;
    const initial = await subtitles.refresh();
    guard(ticket, initial.sourceId);
    const path = await plugin.dialogs.openFile({ title: role === "secondary" ? "Add Secondary Subtitle" : "Open Subtitle" });
    guard(ticket, initial.sourceId);
    if (!path) return void (status = message("noFile"));
    const converted = await readSubtitle(path);
    guard(ticket, initial.sourceId);
    let first: SubtitleTrack | null = null;
    for (const [part, item] of converted.tracks.entries()) {
      const s = await subtitles.refresh();
      guard(ticket, initial.sourceId);
      guard(ticket, s.sourceId);
      // Uploads and commits; the first track of the file is also selected.
      const track = await subtitles.load(item.bytes, {
        format: item.format, title: trackTitle(path, item), select: first ? "none" : role, ifCurrent: s, signal,
      });
      guard(ticket, s.sourceId);
      first ??= track;
      remember(track.id, { path, part, hash: sha256(item.bytes), cues: cuesOf(item.format, item.bytes) });
    }
    desired = undefined;
    const count = converted.tracks.length;
    const state = subtitles.state?.trackConfirmation;
    status =
      role === "primary"
        ? message(confirmed(count > 1 ? "openedTracks" : "opened", state), [basename(path), count], converted.notes)
        : subtitles.state?.secondary?.id === first?.id
          ? message(confirmed(count > 1 ? "secondaryTracks" : "secondary", state), [label(first!), count], converted.notes)
          : message(confirmed("notSecondary", state), [], converted.notes);
  };
  const load = loadAs("primary");
  /** Show Subtitles (Alt+H): hide the selection, or bring back the last shown track. */
  const toggleShown: Action = async (ticket) => {
    const s = await subtitles.refresh();
    if (s.selected) return select(null, s.sourceId)(ticket);
    // Show Subtitles is the primary only: it never takes the track shown as the secondary.
    const free = s.tracks.filter((t) => t.id !== s.secondary?.id);
    const track = free.find((t) => t.id === lastShown) ?? free[0];
    if (!track) return void (status = message("noneToShow"));
    await select(track, s.sourceId)(ticket);
    status = message("shown");
  };

  // ------------------------------------------------ D02 Cycle: track 1 → … → N → off → track 1
  /** The cycle step asked for last; quick presses count from it, not from the host's selection. */
  let cycling: { index: number; seq: number } | null = null;
  let cycleSeq = 0;
  function cycle() {
    const s = subtitles.state;
    if (!s?.tracks.length) return;
    const from = cycling?.index ?? (s.selected ? s.tracks.findIndex((t) => t.id === s.selected!.id) : -1);
    // The track shown as the secondary is skipped, so cycling keeps it.
    const skip = s.secondary ? s.tracks.findIndex((t) => t.id === s.secondary!.id) : -2;
    const after = (i: number) => (i >= s.tracks.length - 1 ? -1 : i + 1);
    let index = after(from);
    if (index === skip) index = after(index);
    const seq = ++cycleSeq;
    cycling = { index, seq };
    const track = index < 0 ? null : s.tracks[index]!;
    const count = s.tracks.length;
    void run(message("switching"), async (ticket) => {
      if (seq !== cycleSeq) return; // replaced by a later press before it started
      await select(track, s.sourceId)(ticket);
      status = track
        ? message(confirmed("cycle", subtitles.state?.trackConfirmation), [label(track), index + 1, count])
        : message("cycleOff");
    }).then((text) => {
      if (seq !== cycleSeq) return;
      cycling = null;
      say(text);
    });
  }

  // ------------------------------------------------ A09 Reload Subtitles
  /**
   * Re-read the file of a track this plugin loaded and replace the track when the
   * bytes changed: load the new bytes (which selects them), restore the previous
   * selection when the reloaded track was not the selected one, then remove the old
   * track. Style and sync are the plugin's style layer and stay as they are.
   */
  const reload =
    (id?: string): Action =>
    async (ticket) => {
      const signal = aborter.signal;
      const s = await subtitles.refresh();
      guard(ticket, s.sourceId);
      const target = id === undefined ? s.selected : s.tracks.find((t) => t.id === id);
      if (!target) return void (status = id === undefined ? message("noSelection") : NONE);
      const record = loaded.get(target.id);
      if (!record) return void (status = message(target.kind === "embedded" ? "embeddedNoReload" : "notOurs"));
      const name = basename(record.path);
      const found = await stat(record.path).then((info) => info.isFile(), () => false);
      guard(ticket, s.sourceId);
      if (!found) return void (status = message("reloadNotFound", [name]));
      const converted = await readSubtitle(record.path).catch((e: Error) => {
        const reason = e instanceof Failure ? e.status.id : "";
        if (reason in RELOAD) throw new Failure(RELOAD[reason as FileError]);
        throw new Failure("reloadFailed", [e.message]);
      });
      guard(ticket, s.sourceId);
      const item = converted.tracks[record.part];
      if (!item) return void (status = message("reloadChanged", [name]));
      const hash = sha256(item.bytes);
      if (hash === record.hash) return void (status = message("unchanged"));
      const before = await subtitles.refresh();
      guard(ticket, s.sourceId);
      const previous = before.selected;
      // A reloaded secondary comes back as the secondary; the primary is untouched.
      const secondary = before.secondary?.id === target.id;
      const track = await subtitles.load(item.bytes, {
        format: item.format, title: target.title || trackTitle(record.path, item), ifCurrent: before, signal,
        ...(secondary ? { select: "secondary" as const } : {}),
      });
      guard(ticket, s.sourceId);
      forget(target.id);
      remember(track.id, { path: record.path, part: record.part, hash, cues: cuesOf(item.format, item.bytes) });
      if (!secondary && previous?.id !== target.id) {
        const now = await subtitles.refresh();
        guard(ticket, s.sourceId);
        await subtitles.select(previous ? (now.tracks.find((t) => t.id === previous.id) ?? null) : null, { ifCurrent: now });
      }
      const after = await subtitles.refresh();
      guard(ticket, s.sourceId);
      const old = after.tracks.find((t) => t.id === target.id);
      if (old) await subtitles.remove(old, { ifCurrent: after });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = id === undefined ? message("reloaded", [name]) : message("autoReloaded");
    };

  // Reload automatically when the subtitle file changes: watch each loaded file's folder (editors often replace the file).
  const watchers = new Map<string, FSWatcher>();
  const reloadTimers = new Map<string, ReturnType<typeof setTimeout>>();
  function unwatch(id: string) {
    watchers.get(id)?.close();
    watchers.delete(id);
    clearTimeout(reloadTimers.get(id));
    reloadTimers.delete(id);
  }
  function watchLoaded(id: string) {
    const record = loaded.get(id);
    if (!autoReload || !record || watchers.has(id)) return;
    const name = basename(record.path);
    try {
      const watcher = watch(dirname(record.path), { persistent: false }, (_event, changed) => {
        if (changed !== null && String(changed) !== name) return;
        clearTimeout(reloadTimers.get(id));
        // Reload once the changes stop for 500 ms.
        const timer = setTimeout(() => {
          reloadTimers.delete(id);
          if (!autoReload || !loaded.has(id)) return;
          void run(message("reloading"), reload(id)).then((text) => {
            if (text.id && text.id !== "unchanged" && text.id !== "cancelled") say(text);
          });
        }, 500);
        timer.unref?.();
        reloadTimers.set(id, timer);
      });
      watcher.on("error", () => unwatch(id));
      watchers.set(id, watcher);
    } catch {
      // A folder that cannot be watched: the manual Reload Subtitles still works.
    }
  }
  function remember(id: string, record: Loaded) {
    loaded.set(id, record);
    watchLoaded(id);
  }
  function forget(id: string) {
    loaded.delete(id);
    unwatch(id);
  }
  function setAutoReload(on: boolean) {
    autoReload = on;
    save();
    if (on) for (const id of loaded.keys()) watchLoaded(id);
    else for (const id of [...watchers.keys()]) unwatch(id);
  }

  /** Why the selected track cannot be styled, or null when it can. */
  function styleBlocked(s = subtitles.state): Status | null {
    if (!s?.selected) return message("selectFirst");
    if (!s.selected.styleSupported) return s.selected.styleReason ? raw(s.selected.styleReason) : message("notStylable");
    if (!s.fonts[0]) return message("noFont");
    return null;
  }
  /** The style in effect (or being edited), with every optional field filled in. */
  const current = (): FullStyle => {
    const s = subtitles.state;
    const style = desired ?? s?.style ?? { fontId: s?.fonts[0]?.id ?? "", ...BASE };
    return { ...BASE_FIELDS, ...style, secondaryPosition } as FullStyle;
  };
  /**
   * Edits merge for 100 ms and apply only to the track selected when the edit
   * began. Returns the edited style, or null (with the reason in `status`). `remember: false`
   * (the secondary position, which lives in settings.json) leaves style.json alone.
   */
  function styleChange(patch: (style: SubtitleStyle) => Partial<SubtitleStyle>, remember = true): SubtitleStyle | null {
    const s = subtitles.state;
    const blocked = styleBlocked(s);
    if (blocked || !s) {
      status = blocked ?? NONE;
      return null;
    }
    const base = current();
    desired = { ...base, ...patch(base) };
    if (remember) {
      saveStyleFile({ style: saveable(desired) });
      restorePending = false;
    }
    editing ??= s;
    const requested = { ...desired }, pinned = editing;
    clearTimeout(styleTimer);
    styleTimer = setTimeout(() => {
      stopStyleTimer();
      void run(message("applyingStyle"), async (ticket) => {
        guard(ticket, pinned.sourceId);
        await subtitles.setStyle(requested, { ifCurrent: pinned });
        guard(ticket, pinned.sourceId);
        status = message(confirmed("styleUpdated", subtitles.state?.styleConfirmation));
      });
    }, 100);
    return desired;
  }
  const syncBy = (steps: number) =>
    styleChange((style) => ({ offsetMs: clamp(style.offsetMs + steps * syncStep, -SYNC_LIMIT, SYNC_LIMIT) }));
  const syncReset = () => styleChange(() => ({ offsetMs: 0 }));
  const sizeBy = (steps: number) =>
    styleChange((s) => ({ fontSize: clamp(s.fontSize + Math.sign(steps) * SIZE_STEP, 16, 72) }));
  const moveBy = (steps: number) =>
    styleChange((s) => ({ marginPercent: clamp(s.marginPercent + Math.sign(steps), 2, 15) }));
  const FIELD_DEFAULTS: Record<string, (style: SubtitleStyle) => Partial<SubtitleStyle>> = {
    size: () => ({ fontSize: BASE.fontSize }),
    position: () => ({ marginPercent: BASE.marginPercent }),
    sync: () => ({ offsetMs: 0 }),
    outline: () => ({ outline: BASE.outline }),
    margin: () => ({ marginPercent: BASE.marginPercent }),
    color: () => ({ color: BASE.color }),
    font: () => ({ fontId: subtitles.state?.fonts[0]?.id ?? "" }),
    bold: () => ({ bold: false }),
    shadow: () => ({ shadow: 0 }),
    box: () => ({ box: null }),
    boxOpacity: (s) => (s.box === null || s.box === undefined ? {} : { box: DEFAULT_BOX }),
  };
  const resetStyle = () => {
    if (busy) return;
    stopStyleTimer();
    return run(message("resetting"), async (ticket) => {
      const s = await subtitles.refresh();
      await subtitles.setStyle(null, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      saveStyleFile({ style: null, shadowLast: DEFAULT_SHADOW, boxOpacity: DEFAULT_BOX });
      restorePending = false;
      positionApplied = ""; // the position is not a style default: it comes back with the next secondary
      status = message("styleReset");
    });
  };

  const common = () => {
    const s = subtitles.state;
    const blocked = styleBlocked(s);
    return { busy, canStyle: !blocked, styleReason: statusText(plugin.i18n, blocked ?? NONE), status: statusText(plugin.i18n, status) };
  };
  tune.bind(() => {
    const s = subtitles.state, selected = s?.selected ?? null, style = current();
    return {
      ...common(),
      trackLabels: [text("Subtitles Off"), ...(s?.tracks.map(label) ?? [])],
      selectedTrack: selected && s ? s.tracks.findIndex((t) => t.id === selected.id) + 1 : 0,
      canRemove: !!selected?.owned,
      secondaryLabels: [text("Secondary Off"), ...(s?.tracks.map(label) ?? [])],
      selectedSecondary: s?.secondary ? s.tracks.findIndex((t) => t.id === s.secondary!.id) + 1 : 0,
      subtitleSize: style.fontSize,
      marginPercent: style.marginPercent,
      syncSeconds: seconds(style.offsetMs),
    };
  });
  syncWindow.bind(() => ({
    ...common(),
    valueText: String(current().offsetMs),
  }));
  settings.bind(() => {
    const s = subtitles.state, style = current();
    const ass = /^(ass|ssa)$/.test(s?.selected?.codec ?? "") && style.assOverride !== "force";
    return {
      ...common(),
      assNote: ass,
      bold: style.bold === true,
      shadowSize: style.shadow ?? 0,
      boxOn: style.box !== null && style.box !== undefined,
      boxOpacity: style.box ?? styleFile.boxOpacity,
      assIndex: assMode() === "force" ? 1 : 0,
      encodingIndex: Math.max(0, ENCODINGS.findIndex((e) => e.id === fallbackEncoding)),
      fontLabels: s?.fonts.map((f) => f.family) ?? [],
      selectedFont: Math.max(0, s?.fonts.findIndex((f) => f.id === style.fontId) ?? 0),
      selectedColor: Math.max(0, COLORS.indexOf(style.color)),
      outlineSize: style.outline,
      marginPercent: style.marginPercent,
      syncStepIndex: Math.max(0, SYNC_STEPS.findIndex((step) => step === syncStep)),
      autoReload,
    };
  });

  const selectIndex = (index: number) => {
    const s = subtitles.state;
    if (busy || !Number.isInteger(index)) return;
    const track = index === 0 ? null : s?.tracks[index - 1];
    if (track !== undefined) return run(message("switching"), select(track, s?.sourceId));
  };
  const removeSelected = () => {
    const track = subtitles.state?.selected, source = subtitles.state?.sourceId;
    if (busy || !track?.owned) return;
    return run(message("removing"), async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source);
      if (s.selected?.id !== track.id) throw new Failure("selectionChanged");
      await subtitles.remove(track, { ifCurrent: s });
      forget(track.id);
      guard(ticket, s.sourceId);
      desired = undefined;
      status = message("removed");
    });
  };
  const resetField = (field: string) => {
    const patch = FIELD_DEFAULTS[String(field)];
    if (patch) styleChange(patch);
  };
  const number = (handler: (value: number) => unknown) => (value: number) => Number.isFinite(value) && handler(value);

  tune.on("selectTrack", selectIndex);
  tune.on("selectSecondary", (index: number) => {
    const s = subtitles.state;
    if (busy || !Number.isInteger(index)) return;
    const track = index === 0 ? null : s?.tracks[index - 1];
    if (track !== undefined) return run(message("switchingSecondary"), selectSecondary(track, s?.sourceId));
  });
  tune.on("loadFile", () => busy || run(message("chooseFile"), load));
  tune.on("cancel", cancel);
  tune.on("sizeStep", number(sizeBy));
  tune.on("positionStep", number(moveBy));
  tune.on("syncStep", number((v) => syncBy(Math.sign(v))));
  tune.on("openSync", () => showSync());
  tune.on("resetField", resetField);
  tune.on("removeSelected", removeSelected);
  tune.on("openSettings", () => showSettings());

  // Ranges are the host's accepted style limits; anything wider fails as `invalid`.
  settings.on("changeOutline", number((v) => styleChange(() => ({ outline: clamp(v, 0, 4) }))));
  settings.on("changeMargin", number((v) => styleChange(() => ({ marginPercent: clamp(v, 2, 15) }))));
  settings.on("sliderReleased", () => {
    const style = current();
    settings.set({
      outlineSize: style.outline, marginPercent: style.marginPercent,
      shadowSize: style.shadow ?? 0, boxOpacity: style.box ?? styleFile.boxOpacity,
    });
  });
  // D09, D11, D17: the same edits as the menu toggles.
  settings.on("changeBold", (on: boolean) => styleChange(() => ({ bold: on === true })));
  settings.on("changeShadow", number((v) => {
    const shadow = clamp(v, 0, 4);
    if (shadow > 0) saveStyleFile({ shadowLast: shadow });
    styleChange(() => ({ shadow }));
  }));
  settings.on("changeBox", (on: boolean) => styleChange(() => ({ box: on === true ? styleFile.boxOpacity : null })));
  settings.on("changeBoxOpacity", number((v) => {
    const opacity = clamp(Math.round(v / 5) * 5, 0, 100);
    saveStyleFile({ boxOpacity: opacity });
    if (current().box !== null) styleChange(() => ({ box: opacity }));
  }));
  settings.on("changeAssOverride", number((v) => {
    if (v === 0 || v === 1) setAssOverride(v === 0 ? "keep" : "force");
  }));
  settings.on("changeEncoding", number((v) => {
    const encoding = Number.isInteger(v) ? ENCODINGS[v]?.id : undefined;
    if (!encoding || encoding === fallbackEncoding) return;
    fallbackEncoding = encoding;
    save();
  }));
  settings.on("changeFont", number((v) => {
    const font = Number.isInteger(v) ? subtitles.state?.fonts[v] : undefined;
    if (font) styleChange(() => ({ fontId: font.id }));
  }));
  settings.on("changeColor", number((v) => {
    const color = Number.isInteger(v) ? COLORS[v] : undefined;
    if (color) styleChange(() => ({ color }));
  }));
  settings.on("resetField", resetField);
  settings.on("resetStyle", resetStyle);
  settings.on("changeSyncStep", number((v) => {
    const step = Number.isInteger(v) ? SYNC_STEPS[v] : undefined;
    if (!step || step === syncStep) return;
    syncStep = step;
    save();
    syncSyncMenu();
  }));
  settings.on("changeAutoReload", (on: boolean) => setAutoReload(on === true));

  // Adjust Sync (D05): Enter or Apply applies; ↑/↓ in the field step by the sync step and apply.
  const applySync = (value: number) => {
    const style = styleChange(() => ({ offsetMs: value }));
    if (style) sayOffset(style.offsetMs);
  };
  syncWindow.on("apply", (text: string) => {
    const parsed = parseSync(String(text));
    if ("error" in parsed) {
      status = parsed.error;
      syncWindow.set({ valueText: String(current().offsetMs) });
      return;
    }
    applySync(parsed.value);
  });
  syncWindow.on("nudge", (text: string, direction: number) => {
    if (!Number.isFinite(direction) || !direction) return;
    const parsed = parseSync(String(text));
    const base = "error" in parsed ? current().offsetMs : parsed.value;
    const value = clamp(base + Math.sign(direction) * syncStep, -SYNC_LIMIT, SYNC_LIMIT);
    applySync(value);
    syncWindow.set({ valueText: String(value) });
  });
  plugin.onError((e) => {
    console.error(`[subtitles] ${e.message}`);
    status = errorText(e);
  });

  async function showTune() {
    await subtitles.refresh();
    await tune.show();
  }
  async function showSettings() {
    await subtitles.refresh();
    await settings.show();
  }
  async function showSync() {
    await subtitles.refresh();
    // Opening fills in the current value, replacing whatever was typed last time.
    syncWindow.set({ valueText: String(current().offsetMs) });
    await syncWindow.show();
  }

  // ---------------------------------------------------------------- menu
  const idle = { enabled: () => !busy, busy: () => busy, reason: () => (busy ? BUSY : null) };
  plugin.commands.add({
    id: "load", path: ["@open", "Open Subtitle…"], order: 113, group: "open", bindings: ["Alt+O"], ...idle,
    run: () => busy || announce(message("chooseFile"), load),
  });
  const hasMedia = () => plugin.player.state.phase === "ready" && !!plugin.player.state.source;
  plugin.commands.add({
    id: "load-secondary", path: ["@open", "Add Secondary Subtitle…"], order: 114, group: "open",
    enabled: () => !busy && hasMedia(), busy: () => busy,
    reason: () => (busy ? BUSY : hasMedia() ? null : NO_MEDIA),
    run: () => busy || announce(message("chooseSecondaryFile"), loadAs("secondary")),
  });
  plugin.commands.add({
    id: "show", path: ["@subtitles", "Show Subtitles"], order: 301, bindings: ["Alt+H"], ...idle,
    checked: () => !!subtitles.state?.selected,
    run: () => busy || announce(message("togglingShown"), toggleShown),
  });
  plugin.commands.add({
    id: "track-off", path: ["@subtitles", "Subtitle Track", "Off"], order: 302, group: "subtitle-track", radio: true, ...idle,
    checked: () => !subtitles.state?.selected,
    run: () => busy || announce(message("hiding"), select(null)),
  });
  // A08 Subtitles ▸ Secondary Subtitle ▸: Off (312), the tracks (313–320), Position ▸ (321).
  plugin.commands.add({
    id: "second-off", path: ["@subtitles", "Secondary Subtitle", "Off"], order: 312, group: "subtitle-second", radio: true, ...idle,
    checked: () => !subtitles.state?.secondary,
    run: () => busy || announce(message("closingSecondary"), selectSecondary(null)),
  });
  for (const { id, label: title } of SECONDARY_POSITIONS)
    plugin.commands.add({
      id: `second-position-${id}`, path: ["@subtitles", "Secondary Subtitle", "Position", title], order: 321, group: "subtitle-second-pos",
      radio: true, checked: () => secondaryPosition === id,
      run: () => setSecondaryPosition(id),
    });
  // The track items are rebuilt in a child scope whenever the track list changes.
  let trackMenu = plugin.scope();
  let secondMenu = plugin.scope();
  let trackKey = "";
  let rebuilding = Promise.resolve();
  /**
   * The track items' titles and menu keys (the English rendering; menus merge by it).
   * A path segment is 1–80 bytes without control characters or edge spaces; keys must differ.
   */
  function menuTitles(tracks: SubtitleTrack[]) {
    const used = new Set<string>();
    return tracks.slice(0, MENU_TRACKS).map((t, i) => {
      let title: string | Text, key: string;
      if (t.title) {
        key = clean(titled(t)).slice(0, 24);
        title = key || text("Track {}", [i + 1]);
        key ||= `Track ${i + 1}`;
      } else {
        const language = clean(t.language ?? "").slice(0, 16);
        const english = t.kind === "embedded" ? `Embedded ${t.embeddedId ?? ""}` : "External subtitle";
        title = language ? text("{} · {}", [fallbackName(t), language]) : fallbackName(t);
        key = language ? `${english} · ${language}` : english;
      }
      if (used.has(key) || RESERVED.has(key)) {
        title = typeof title === "string" ? `${title} (${i + 1})` : text("{} ({})", [title, i + 1]);
        key = `${key} (${i + 1})`;
      }
      used.add(key);
      return { title, key };
    });
  }
  function syncTrackMenu(s: SubtitlesState) {
    const titles = menuTitles(s.tracks);
    const key = JSON.stringify(s.tracks.slice(0, MENU_TRACKS).map((t, i) => [t.id, titles[i]!.key, t.text]));
    if (key === trackKey) return;
    trackKey = key;
    const tracks = s.tracks.slice(0, MENU_TRACKS);
    // Without any text track the Secondary Subtitle menu is only Off.
    const seconds = s.tracks.some((t) => t.text) ? tracks : [];
    rebuilding = rebuilding.then(async () => {
      await trackMenu.dispose();
      await secondMenu.dispose();
      trackMenu = plugin.scope();
      secondMenu = plugin.scope();
      seconds.forEach((track, i) =>
        secondMenu.commands.add({
          // Track names are data; a fallback name is a text value (menuTitles).
          id: `second-${i + 1}`, path: ["@subtitles", "Secondary Subtitle", /* i18n-ignore */ titles[i]!.title], order: 313 + i,
          group: "subtitle-second", radio: true, busy: () => busy,
          enabled: () => !busy && !secondBlocked(track),
          reason: () => (busy ? BUSY : secondBlocked(track)),
          checked: () => track.id === subtitles.state?.secondary?.id,
          run: () => busy || announce(message("switchingSecondary"), selectSecondary(track, s.sourceId)),
        }),
      );
      tracks.forEach((track, i) =>
        trackMenu.commands.add({
          id: `track-${i + 1}`, path: ["@subtitles", "Subtitle Track", /* i18n-ignore */ titles[i]!.title], order: 303 + i,
          group: "subtitle-track", radio: true, ...idle,
          checked: () => track.id === subtitles.state?.selected?.id,
          run: () => busy || announce(message("switching"), select(track, s.sourceId)),
        }),
      );
    });
  }
  plugin.commands.add({
    id: "track-cycle", path: ["@subtitles", "Subtitle Track", "Cycle"], order: 311, group: "subtitle-cycle", bindings: ["Alt+L"],
    enabled: () => !!subtitles.state?.tracks.length,
    reason: () => (subtitles.state?.tracks.length ? null : "No subtitle tracks"),
    run: cycle,
  });

  // Subtitle Style ▸ (D06, D07): same edits as the Subtitle Tuning buttons, reported on screen.
  /**
   * Key-driven style edits: never blocked by a running operation, so held keys keep repeating.
   * `text` shows the result on screen (`osd.show` with the edited style's values).
   */
  const styleRun = (apply: () => SubtitleStyle | null, text: (style: FullStyle) => Promise<void>) => () => {
    const style = apply();
    if (style) shown(text({ ...BASE_FIELDS, ...style } as FullStyle));
    else say(status);
    plugin.update();
  };
  const STYLE = ["@subtitles", "Subtitle Style"] as const;
  plugin.commands.add({
    id: "size-up", path: [...STYLE, "Size +"], order: 325, group: "style-font", bindings: ["Alt+PageUp"], repeat: true,
    run: styleRun(() => sizeBy(1), (s) => osd.show("Subtitle size: larger ({})", [s.fontSize])),
  });
  plugin.commands.add({
    id: "size-down", path: [...STYLE, "Size −"], order: 326, group: "style-font", bindings: ["Alt+PageDown"], repeat: true,
    run: styleRun(() => sizeBy(-1), (s) => osd.show("Subtitle size: smaller ({})", [s.fontSize])),
  });
  plugin.commands.add({
    id: "move-up", path: [...STYLE, "Move Up"], order: 330, group: "style-move", bindings: ["Alt+ArrowUp"], repeat: true,
    run: styleRun(() => moveBy(1), (s) => osd.show("Subtitle position: up ({}%)", [s.marginPercent])),
  });
  plugin.commands.add({
    id: "move-down", path: [...STYLE, "Move Down"], order: 331, group: "style-move", bindings: ["Alt+ArrowDown"], repeat: true,
    run: styleRun(() => moveBy(-1), (s) => osd.show("Subtitle position: down ({}%)", [s.marginPercent])),
  });
  plugin.commands.add({
    id: "move-reset", path: [...STYLE, "Reset Position"], order: 334, group: "style-move", bindings: ["Alt+Home"], repeat: false,
    run: styleRun(
      () => styleChange(() => ({ marginPercent: BASE.marginPercent })),
      (s) => osd.show("Subtitle position: default ({}%)", [s.marginPercent]),
    ),
  });
  // D09 Bold, D11 Shadow / Opaque Box (toggles; the window has the sliders).
  plugin.commands.add({
    id: "bold", path: [...STYLE, "Bold"], order: 324, group: "style-font", bindings: ["Alt+B"], repeat: false,
    checked: () => current().bold,
    run: styleRun(
      () => styleChange((s) => ({ bold: !s.bold })),
      (s) => osd.show(s.bold ? "Bold subtitles" : "Bold subtitles off"),
    ),
  });
  plugin.commands.add({
    id: "shadow", path: [...STYLE, "Shadow"], order: 335, group: "style-box", bindings: [], repeat: false,
    checked: () => current().shadow > 0,
    run: styleRun(
      () => styleChange((s) => ({ shadow: (s.shadow ?? 0) > 0 ? 0 : styleFile.shadowLast })),
      (s) => (s.shadow > 0 ? osd.show("Subtitle shadow: on ({})", [s.shadow]) : osd.show("Subtitle shadow: off")),
    ),
  });
  plugin.commands.add({
    id: "box", path: [...STYLE, "Opaque Box"], order: 336, group: "style-box", bindings: [], repeat: false,
    checked: () => current().box !== null,
    run: styleRun(
      () => styleChange((s) => ({ box: s.box === null || s.box === undefined ? styleFile.boxOpacity : null })),
      (s) => osd.show(s.box !== null ? "Opaque subtitle box: on" : "Opaque subtitle box: off"),
    ),
  });
  // D17 Original ASS/SSA Styles: the player's ASS override, a style field. Without a stylable
  // track the choice is kept and applied with the remembered style later.
  function setAssOverride(mode: "keep" | "force") {
    if (!styleChange(() => ({ assOverride: mode }))) {
      saveStyleFile({ style: { ...(styleFile.style ?? saveable(current())), assOverride: mode } });
      restorePending = true;
    }
    shown(osd.show(mode === "keep" ? "ASS/SSA animated subtitles: on" : "ASS/SSA animated subtitles: off"));
    plugin.update();
  }
  /** The ASS choice in effect, or the one waiting to be applied. */
  const assMode = () => (restorePending && styleFile.style ? styleFile.style.assOverride : current().assOverride);
  plugin.commands.add({
    id: "ass-original", path: ["@subtitles", "Original ASS/SSA Styles"], order: 337, group: "subtitle-render",
    checked: () => assMode() !== "force",
    run: () => setAssOverride(assMode() === "force" ? "keep" : "force"),
  });
  /** Apply the remembered style (style.json) once a stylable track is selected. */
  function restoreStyle() {
    const saved = styleFile.style;
    if (!restorePending || !saved || styleBlocked()) return;
    const fonts = subtitles.state?.fonts ?? [];
    styleChange((s) => ({ ...saved, fontId: fonts.some((f) => f.id === saved.fontId) ? saved.fontId : s.fontId, offsetMs: s.offsetMs }));
  }

  // A08 secondary subtitle position: a style field, so it rides on every style this plugin sets. A change is
  // applied at once when a style can be set; otherwise (or after Reset All, or on a new source)
  // it is applied once a secondary subtitle is shown on a stylable primary.
  const shownPosition = (s: SubtitlesState) => s.style?.secondaryPosition ?? "top";
  let positionApplied = "";
  function syncPosition(s = subtitles.state) {
    if (!s?.secondary || styleTimer || shownPosition(s) === secondaryPosition || styleBlocked(s)) return;
    const key = `${s.sourceId}|${s.selected?.id}|${secondaryPosition}`;
    if (positionApplied === key) return; // once: another plugin's style layer is not fought over
    positionApplied = key;
    styleChange(() => ({}), false);
  }
  function setSecondaryPosition(position: SecondaryPosition) {
    const changed = position !== secondaryPosition;
    secondaryPosition = position;
    if (changed) save();
    const s = subtitles.state;
    if (s && !styleBlocked(s) && shownPosition(s) !== position) {
      positionApplied = `${s.sourceId}|${s.selected?.id}|${position}`;
      styleChange(() => ({}), false);
    }
    shown(
      osd.show(position === "top" ? "Secondary subtitle position: top" : "Secondary subtitle position: above the main subtitle"),
    );
    plugin.update();
  }

  // B11 Previous / Next Dialogue / Current Subtitle Start: cues of the selected text track. Tracks this
  // plugin loaded are parsed from the uploaded bytes; embedded ones are read with ffmpeg.
  let cueCache: { key: string; cues: Cue[] } | null = null;
  let cueReading = false;
  /**
   * Jumps run one after another, each from the position its predecessor's seek
   * left (the SDK refreshes the state after a seek). A held key repeats faster
   * than seeks land: one press may wait behind the running jump, further ones are dropped.
   */
  let jumps = Promise.resolve();
  let waiting = 0;
  const cueKey = (s: SubtitlesState, track: SubtitleTrack) => `${s.sourceId}:${track.id}`;
  function dialogueBlocked() {
    const p = plugin.player.state;
    if (!p.source || !p.can.seek) return NO_SEEK;
    const s = subtitles.state, track = s?.selected;
    if (!s || !track) return NO_TEXT;
    const own = loaded.get(track.id);
    if (own) return own.cues.length ? null : NO_TEXT;
    if (!track.text || track.kind !== "embedded" || track.streamIndex === null || isRemote(p.source)) return NO_TEXT;
    if (cueCache?.key === cueKey(s, track) && !cueCache.cues.length) return NO_TEXT;
    return null;
  }
  async function readCues(s: SubtitlesState, track: SubtitleTrack, source: string): Promise<Cue[]> {
    const own = loaded.get(track.id);
    if (own) return own.cues;
    const key = cueKey(s, track);
    if (cueCache?.key === key) return cueCache.cues;
    const size = await stat(source).then((info) => info.size, () => 0);
    // About 1 s per 100 MB of media (ffmpeg reads the whole file), at least 15 s.
    const timeoutMs = Math.max(15_000, Math.ceil(size / 100_000_000) * 1000);
    const slow = setTimeout(() => shown(osd.show("Reading subtitles…")), 500);
    cueReading = true; // the jump commands show busy meanwhile
    plugin.update();
    try {
      const { stdout } = await plugin.tools.run(
        "ffmpeg",
        ["-v", "error", "-nostdin", "-i", source, "-map", `0:${track.streamIndex}`, "-f", "srt", "pipe:1"],
        { timeoutMs, signal: aborter.signal },
      );
      const cues = parseSrt(new TextDecoder().decode(stdout));
      if (subtitles.state?.sourceId === s.sourceId) cueCache = { key, cues };
      return cues;
    } finally {
      clearTimeout(slow);
      cueReading = false;
      plugin.update();
    }
  }
  const cueError = (e: unknown) => {
    const error = e as { code?: string; name?: string };
    if (error?.name === "AbortError" || error?.code === "aborted") return null;
    if (error?.code === "unavailable") return "ffmpeg is needed to read embedded subtitles";
    if (error?.code === "timeout") return "Reading embedded subtitles timed out";
    return "Cannot read embedded subtitles";
  };
  const JUMP_TEXT = {
    previous: ["Seek: previous dialogue {}", "No previous dialogue"],
    next: ["Seek: next dialogue {}", "No next dialogue"],
    current: ["Seek: start of current dialogue {}", "No dialogue to jump to"],
  } as const satisfies Record<Jump, readonly [string, string]>;
  function jump(kind: Jump) {
    if (waiting >= 2) return; // one running, one waiting
    waiting++;
    jumps = jumps.then(() => jumpOnce(kind)).finally(() => void waiting--);
    return jumps;
  }
  async function jumpOnce(kind: Jump) {
    const blocked = dialogueBlocked();
    if (blocked) return shown(osd.show(blocked));
    const s = subtitles.state!, track = s.selected!, p = plugin.player.state;
    let cues: Cue[];
    try {
      cues = await readCues(s, track, p.source!);
    } catch (e) {
      const text = cueError(e);
      if (text) shown(osd.show(text));
      return;
    }
    if (plugin.player.state.sourceId !== p.sourceId) return; // the media changed while reading
    if (!cues.length) return shown(osd.show(NO_TEXT));
    // Start from where the previous jump's seek landed, not an event that has not arrived yet.
    const now = await plugin.player.refresh().catch(() => plugin.player.state);
    if (now.sourceId !== p.sourceId) return;
    const target = dialogueTarget(cues, now.position ?? 0, current().offsetMs, kind);
    const [done, none] = JUMP_TEXT[kind];
    if (target === null) return shown(osd.show(none));
    shown(osd.show(done, [clock(target)]));
    // A little before the start, so decoding does not land on the second half of the cue.
    await plugin.player.seek(Math.max(0, target - 0.05)).catch((e: { code?: string }) => {
      if (e?.code !== "stale") say(errorText(e)); // someone changed the source meanwhile: no retry
    });
  }
  const dialogue = { enabled: () => !cueReading && !dialogueBlocked(), busy: () => cueReading, reason: () => (cueReading ? "Reading subtitles…" : dialogueBlocked()) };
  plugin.commands.add({
    id: "dialogue-previous", path: ["@playback", "Seek", "Previous Dialogue"], order: 217, group: "jump-subtitle",
    bindings: ["Home"], repeat: true, ...dialogue, run: () => jump("previous"),
  });
  plugin.commands.add({
    id: "dialogue-next", path: ["@playback", "Seek", "Next Dialogue"], order: 217, group: "jump-subtitle",
    bindings: ["End"], repeat: true, ...dialogue, run: () => jump("next"),
  });
  plugin.commands.add({
    id: "dialogue-current", path: ["@playback", "Seek", "Current Subtitle Start"], order: 217, group: "jump-subtitle",
    bindings: ["Ctrl+Home"], ...dialogue, run: () => jump("current"),
  });

  // Sync ▸: the Earlier / Later titles show the sync step, so they live in a child scope rebuilt when it changes.
  let syncMenu = plugin.scope();
  let syncMenuStep = 0;
  let syncRebuild = Promise.resolve();
  const offsetText = (s: SubtitleStyle) => {
    sayOffset(s.offsetMs);
    return Promise.resolve();
  };
  function syncSyncMenu() {
    if (syncMenuStep === syncStep) return;
    const step = (syncMenuStep = syncStep);
    const titles = SYNC_MENU.find((row) => row.step === step) ?? SYNC_MENU[3];
    syncRebuild = syncRebuild.then(async () => {
      await syncMenu.dispose();
      syncMenu = plugin.scope();
      syncMenu.commands.add({
        id: "sync-earlier", path: ["@subtitles", "Sync", titles.earlier], order: 339, group: "subtitle-sync",
        bindings: ["."], repeat: true, run: styleRun(() => syncBy(-1), offsetText),
      });
      syncMenu.commands.add({
        id: "sync-later", path: ["@subtitles", "Sync", titles.later], order: 340, group: "subtitle-sync",
        bindings: [","], repeat: true, run: styleRun(() => syncBy(1), offsetText),
      });
    });
  }
  syncSyncMenu();
  plugin.commands.add({
    id: "sync-reset", path: ["@subtitles", "Sync", "Reset"], order: 341, group: "subtitle-sync",
    bindings: ["/"], repeat: true, run: styleRun(syncReset, offsetText),
  });
  plugin.commands.add({
    id: "sync-input", path: ["@subtitles", "Sync", "Adjust Sync…"], order: 342, group: "subtitle-sync-input", run: showSync,
  });
  const TOOLS = {
    reload: "Reload Subtitles",
    refresh: "Refresh Subtitle Tracks",
    tune: "Subtitle Tuning…",
    settings: "Subtitle Settings…",
  } as const;
  type Extra = Partial<Pick<CommandOptions, "bindings"> & typeof idle>;
  const tool = (id: keyof typeof TOOLS, order: number, run: () => unknown, extra: Extra = {}) =>
    plugin.commands.add({ id, path: ["@subtitles", TOOLS[id]], order, group: "subtitle-tools", run, ...extra });
  tool("reload", 370, () => busy || announce(message("reloading"), reload()), { bindings: ["Ctrl+Alt+Y"], ...idle });
  tool("refresh", 371, () => busy || announce(message("refreshing"), refresh));
  tool("tune", 372, showTune);
  tool("settings", 373, showSettings);

  // A new source cancels pending work; a new source or selection drops the unconfirmed style.
  subtitles.onChange((s) => {
    if (seen && s.sourceId > seen.sourceId) {
      cancel();
      status = message("mediaSwitched");
    }
    if (!seen || s.sourceId !== seen.sourceId || s.selected?.id !== seen.selected?.id) desired = undefined;
    if (s.selected) lastShown = s.selected.id;
    // Loaded tracks belong to their source; forget the ones that are gone.
    for (const id of [...loaded.keys()]) if (!s.tracks.some((t) => t.id === id)) forget(id);
    seen = s;
    syncTrackMenu(s);
    restoreStyle();
    syncPosition(s);
  });
  // The host has no subtitle event; an open window also re-reads every 1.5 s.
  const poll = setInterval(() => {
    if ((tune.opened || settings.opened || syncWindow.opened) && !busy)
      subtitles.refresh().catch((e) => {
        status = errorText(e);
        plugin.update();
      });
  }, 1500);
  poll.unref();
  plugin.onDispose(() => {
    clearInterval(poll);
    stopStyleTimer();
    for (const id of [...watchers.keys()]) unwatch(id);
  });
  syncTrackMenu(await subtitles.refresh());
  restoreStyle();
  await rebuilding;
  await syncRebuild;
}

/** A JSON file in the data directory; without one, settings last for this run only. */
function store<T extends Record<string, unknown>>(plugin: Plugin, name: string, defaults: T) {
  try {
    return plugin.storage.json<T>(name, { defaults, validate: (v): v is T => isObject(v) });
  } catch {
    let value = defaults;
    return { get value() { return value; }, save(next: T) { value = next; } };
  }
}
