import { createHash } from "node:crypto";
import { watch, readFileSync, renameSync, writeFileSync } from "node:fs";
import type { FSWatcher } from "node:fs";
import { stat } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import type {
  CommandOptions,
  Plugin,
  Scope,
  SubtitleStyle,
  SubtitleTrack,
  SubtitlesState,
} from "../../sdk/plugin/index.ts";
import { readSrt } from "./srt.ts";
import { SYNC_LIMIT, SYNC_STEPS, parse, parseSync, serialize, stepSeconds } from "./model.ts";

const COLORS = ["white", "yellow", "cyan"] as const;
/** 字幕 ▶ 选择字幕轨 lists at most this many tracks (orders 303–310); the rest are in 字幕微调. */
const MENU_TRACKS = 8;
const SIZE_STEP = 2;
/** Titles the track items must not take: the fixed items of 选择字幕轨 ▶. */
const RESERVED = new Set(["不显示", "依次切换"]);
const BASE = { fontSize: 36, color: "white", outline: 2, marginPercent: 5, offsetMs: 0 } as const;
const CANCELLED = "操作已取消或媒体已切换。";
const STALE = "字幕轨已变化，请重新调整。";

/** One short Chinese sentence for any failure; host details go to the log only. */
function errorText(e: unknown) {
  const error = e as { code?: string; name?: string; message?: string };
  if (error?.name === "AbortError") return CANCELLED;
  switch (error?.code) {
    case undefined:
      return error?.message ?? String(e); // our own messages are already Chinese
    case "stale":
      return STALE;
    case "invalid":
      return "播放器拒绝了该字幕（文件过大或内容无效）。";
    case "busy":
      return "播放器忙，请稍后再试。";
    case "unavailable":
    case "unsupported":
      return "当前媒体不支持此操作。";
    case "backend":
      return "播放器无法载入此字幕。";
    default:
      return "字幕操作失败。";
  }
}
const label = (t: SubtitleTrack) =>
  `${t.title || (t.kind === "embedded" ? `内嵌 ${t.embeddedId}` : "外部字幕")}${t.language ? ` · ${t.language}` : ""}`;
const seconds = (ms: number) => `${ms > 0 ? "+" : ms < 0 ? "−" : ""}${Math.abs(ms) / 1000}`;
/** OSD wording for a subtitle offset (positive = later). */
const syncText = (ms: number) =>
  ms === 0 ? "字幕同步: 0 秒" : `字幕同步: ${ms < 0 ? "超前" : "滞后"} ${Math.abs(ms) / 1000} 秒`;
const confirmation = (state: "waiting" | "applied" | "uncertain" | undefined) =>
  state === "waiting" ? "（等待生效）" : state === "uncertain" ? "（未能确认是否生效）" : "";
const clamp = (value: number, min: number, max: number) => Math.round(Math.max(min, Math.min(max, value)));

type Action = (ticket: number) => Promise<void>;
/** A track this plugin loaded: where it came from, to reload it (A09). Memory only. */
type Loaded = { path: string; hash: string };
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** Subtitles: 字幕微调 and 字幕设置 windows, 字幕 ▶ menu, serialized subtitle operations. */
export default async function activate(plugin: Plugin) {
  const { subtitles, osd } = plugin;
  const tune = plugin.windows.create("ui/tune.slint", { id: "subtitle-tune" });
  const settings = plugin.windows.create("ui/settings.slint", { id: "subtitle-settings" });
  const syncWindow = plugin.windows.create("ui/sync.slint", { id: "subtitle-sync" });
  const file = settingsFile(plugin);
  const stored = parse(readJson(file));
  let syncStep = stored.syncStepMs;
  let autoReload = stored.autoReload;
  const save = () => {
    if (!file) return;
    try {
      writeFileSync(`${file}.tmp`, JSON.stringify(serialize({ ...stored, syncStepMs: syncStep, autoReload }), null, 2));
      renameSync(`${file}.tmp`, file);
    } catch (e) {
      console.error(`[subtitles] settings not saved: ${(e as Error).message}`);
    }
  };
  /** Tracks this plugin loaded on the current source, by track id. */
  const loaded = new Map<string, Loaded>();
  let status = "";
  let busy = false;
  let epoch = 0; // bumped by cancel and media switches; stale actions stop at the next guard
  let aborter = new AbortController(); // aborted together with each epoch bump (stops an upload)
  let chain = Promise.resolve();
  let desired: SubtitleStyle | undefined; // edited style shown before the host confirms it
  let editing: SubtitlesState | null = null; // selection the pending style edit belongs to
  let styleTimer: ReturnType<typeof setTimeout> | undefined;
  let seen: SubtitlesState | null = null;
  let lastShown: string | null = null; // track id that 显示字幕 brings back
  const say = (text: string) => void osd.show(text).catch(() => {});

  const guard = (ticket: number, source?: number) => {
    if (epoch !== ticket || (source !== undefined && source !== subtitles.state?.sourceId))
      throw new Error(CANCELLED);
  };
  /** One operation at a time; the menu shows busy meanwhile. Resolves to the final status. */
  function run(text: string, action: Action) {
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
  const announce = (text: string, action: Action) => run(text, action).then(say);
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
    status = "已取消。";
  }

  const refresh: Action = async () => {
    await subtitles.refresh();
    desired = undefined;
    status = "字幕轨已刷新。";
  };
  const select =
    (track: SubtitleTrack | null, source?: number): Action =>
    async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source ?? s.sourceId);
      await subtitles.select(track, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = track ? `字幕：${label(track)}${confirmation(subtitles.state?.trackConfirmation)}` : "隐藏字幕";
    };
  const load: Action = async (ticket) => {
    const signal = aborter.signal;
    const initial = await subtitles.refresh();
    guard(ticket, initial.sourceId);
    const path = await plugin.dialogs.openFile({ title: "打开字幕" });
    guard(ticket, initial.sourceId);
    if (!path) return void (status = "未选择字幕文件。");
    const bytes = await readSrt(path);
    guard(ticket, initial.sourceId);
    const s = await subtitles.refresh();
    guard(ticket, initial.sourceId);
    guard(ticket, s.sourceId);
    const track = await subtitles.load(bytes, { ifCurrent: s, signal }); // uploads, commits and selects
    guard(ticket, s.sourceId);
    remember(track.id, { path, hash: sha256(bytes) });
    desired = undefined;
    status = `已载入字幕 ${basename(path)}${confirmation(subtitles.state?.trackConfirmation)}`;
  };
  /** 显示字幕 (Alt+H): hide the selection, or bring back the last shown track. */
  const toggleShown: Action = async (ticket) => {
    const s = await subtitles.refresh();
    if (s.selected) return select(null, s.sourceId)(ticket);
    const track = s.tracks.find((t) => t.id === lastShown) ?? s.tracks[0];
    if (!track) return void (status = "没有可显示的字幕。");
    await select(track, s.sourceId)(ticket);
    status = "显示字幕";
  };

  // ------------------------------------------------ D02 依次切换: track 1 → … → N → 关闭 → track 1
  /** The cycle step asked for last; quick presses count from it, not from the host's selection. */
  let cycling: { index: number; seq: number } | null = null;
  let cycleSeq = 0;
  function cycle() {
    const s = subtitles.state;
    if (!s?.tracks.length) return;
    const from = cycling?.index ?? (s.selected ? s.tracks.findIndex((t) => t.id === s.selected!.id) : -1);
    const index = from >= s.tracks.length - 1 ? -1 : from + 1;
    const seq = ++cycleSeq;
    cycling = { index, seq };
    const track = index < 0 ? null : s.tracks[index]!;
    const count = s.tracks.length;
    void run("正在切换字幕轨…", async (ticket) => {
      if (seq !== cycleSeq) return; // replaced by a later press before it started
      await select(track, s.sourceId)(ticket);
      status = track ? `字幕：${label(track)}（${index + 1}/${count}）${confirmation(subtitles.state?.trackConfirmation)}` : "字幕：关闭";
    }).then((text) => {
      if (seq !== cycleSeq) return;
      cycling = null;
      say(text);
    });
  }

  // ------------------------------------------------ A09 重载字幕
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
      if (!target) return void (status = id === undefined ? "没有选中的字幕。" : "");
      const record = loaded.get(target.id);
      if (!record)
        return void (status = target.kind === "embedded" ? "内嵌字幕无需重载" : "此字幕不是由字幕插件载入，无法重载");
      const name = basename(record.path);
      const found = await stat(record.path).then((info) => info.isFile(), () => false);
      guard(ticket, s.sourceId);
      if (!found) return void (status = `无法重载：找不到字幕文件 ${name}`);
      const bytes = await readSrt(record.path).catch((e: Error) => {
        throw new Error(`无法重载：${e.message}`);
      });
      guard(ticket, s.sourceId);
      const hash = sha256(bytes);
      if (hash === record.hash) return void (status = "字幕未改变");
      const before = await subtitles.refresh();
      guard(ticket, s.sourceId);
      const previous = before.selected;
      const track = await subtitles.load(bytes, { ifCurrent: before, signal });
      guard(ticket, s.sourceId);
      forget(target.id);
      remember(track.id, { path: record.path, hash });
      if (previous?.id !== target.id) {
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
      status = id === undefined ? `已重载字幕 ${name}` : "字幕已自动重载";
    };

  // 字幕文件改动后自动重载: watch each loaded file's folder (editors often replace the file).
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
          void run("正在重载字幕…", reload(id)).then((text) => {
            if (text && text !== "字幕未改变" && text !== CANCELLED) say(text);
          });
        }, 500);
        timer.unref?.();
        reloadTimers.set(id, timer);
      });
      watcher.on("error", () => unwatch(id));
      watchers.set(id, watcher);
    } catch {
      // A folder that cannot be watched: the manual 重载字幕 still works.
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
  function styleBlocked(s = subtitles.state) {
    if (!s?.selected) return "请先选择字幕轨。";
    if (!s.selected.styleSupported) return s.selected.styleReason ?? "当前字幕轨不支持调整。";
    if (!s.fonts[0]) return "没有可用的字幕字体。";
    return null;
  }
  const current = () => {
    const s = subtitles.state;
    return desired ?? s?.style ?? { fontId: s?.fonts[0]?.id ?? "", ...BASE };
  };
  /**
   * Edits merge for 100 ms and apply only to the track selected when the edit
   * began. Returns the edited style, or null (with the reason in `status`).
   */
  function styleChange(patch: (style: SubtitleStyle) => Partial<SubtitleStyle>): SubtitleStyle | null {
    const s = subtitles.state;
    const blocked = styleBlocked(s);
    if (blocked || !s) {
      status = blocked ?? "";
      return null;
    }
    const base = current();
    desired = { ...base, ...patch(base) };
    editing ??= s;
    const requested = { ...desired }, pinned = editing;
    clearTimeout(styleTimer);
    styleTimer = setTimeout(() => {
      stopStyleTimer();
      void run("正在应用字幕样式…", async (ticket) => {
        guard(ticket, pinned.sourceId);
        await subtitles.setStyle(requested, { ifCurrent: pinned });
        guard(ticket, pinned.sourceId);
        status = `字幕样式已更新${confirmation(subtitles.state?.styleConfirmation)}`;
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
  };
  const resetStyle = () => {
    if (busy) return;
    stopStyleTimer();
    return run("正在恢复默认样式…", async (ticket) => {
      const s = await subtitles.refresh();
      await subtitles.setStyle(null, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = "已恢复默认样式。";
    });
  };

  const common = () => {
    const s = subtitles.state;
    const blocked = styleBlocked(s);
    return { busy, canStyle: !blocked, styleReason: blocked ?? "", statusText: status };
  };
  tune.bind(() => {
    const s = subtitles.state, selected = s?.selected ?? null, style = current();
    return {
      ...common(),
      trackLabels: ["关闭字幕", ...(s?.tracks.map(label) ?? [])],
      selectedTrack: selected && s ? s.tracks.findIndex((t) => t.id === selected.id) + 1 : 0,
      canRemove: !!selected?.owned,
      sizeText: `字号 ${style.fontSize}`,
      positionText: `位置 ${style.marginPercent}%`,
      syncText: `同步 ${seconds(style.offsetMs)}`,
    };
  });
  syncWindow.bind(() => ({
    ...common(),
    valueText: String(current().offsetMs),
  }));
  settings.bind(() => {
    const s = subtitles.state, style = current();
    return {
      ...common(),
      fontLabels: s?.fonts.map((f) => f.family) ?? [],
      selectedFont: Math.max(0, s?.fonts.findIndex((f) => f.id === style.fontId) ?? 0),
      selectedColor: Math.max(0, COLORS.indexOf(style.color)),
      outlineSize: style.outline,
      marginPercent: style.marginPercent,
      syncStepLabels: SYNC_STEPS.map((step) => `${stepSeconds(step)} 秒`),
      syncStepIndex: Math.max(0, SYNC_STEPS.findIndex((step) => step === syncStep)),
      autoReload,
    };
  });

  const selectIndex = (index: number) => {
    const s = subtitles.state;
    if (busy || !Number.isInteger(index)) return;
    const track = index === 0 ? null : s?.tracks[index - 1];
    if (track !== undefined) return run("正在切换字幕轨…", select(track, s?.sourceId));
  };
  const removeSelected = () => {
    const track = subtitles.state?.selected, source = subtitles.state?.sourceId;
    if (busy || !track?.owned) return;
    return run("正在移除字幕…", async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source);
      if (s.selected?.id !== track.id) throw new Error("选中的字幕已变化，请重新选择。");
      await subtitles.remove(track, { ifCurrent: s });
      forget(track.id);
      guard(ticket, s.sourceId);
      desired = undefined;
      status = "已移除外部字幕。";
    });
  };
  const resetField = (field: string) => {
    const patch = FIELD_DEFAULTS[String(field)];
    if (patch) styleChange(patch);
  };
  const number = (handler: (value: number) => unknown) => (value: number) => Number.isFinite(value) && handler(value);

  tune.on("selectTrack", selectIndex);
  tune.on("loadFile", () => busy || run("请选择字幕文件…", load));
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
    settings.set({ outlineSize: style.outline, marginPercent: style.marginPercent });
  });
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

  // 手动调整同步 (D05): Enter or 应用 applies; ↑/↓ in the field step by the 同步单位 and apply.
  const applySync = (value: number) => {
    const style = styleChange(() => ({ offsetMs: value }));
    if (style) say(syncText(style.offsetMs));
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
  const idle = { enabled: () => !busy, busy: () => busy, reason: () => (busy ? "字幕操作进行中" : null) };
  plugin.commands.add({
    id: "load", path: ["打开", "打开字幕…"], order: 113, group: "open", bindings: ["Alt+O"], ...idle,
    run: () => busy || announce("请选择字幕文件…", load),
  });
  plugin.commands.add({
    id: "show", path: ["字幕", "显示字幕"], order: 301, bindings: ["Alt+H"], ...idle,
    checked: () => !!subtitles.state?.selected,
    run: () => busy || announce("正在切换字幕显示…", toggleShown),
  });
  plugin.commands.add({
    id: "track-off", path: ["字幕", "选择字幕轨", "不显示"], order: 302, group: "subtitle-track", radio: true, ...idle,
    checked: () => !subtitles.state?.selected,
    run: () => busy || announce("正在隐藏字幕…", select(null)),
  });
  // The track items are rebuilt in a child scope whenever the track list changes.
  let trackMenu = plugin.scope();
  let trackKey = "";
  let rebuilding = Promise.resolve();
  function menuTitles(tracks: SubtitleTrack[]) {
    const used = new Set<string>();
    return tracks.slice(0, MENU_TRACKS).map((t, i) => {
      // A path segment is 1–80 bytes without control characters or edge spaces; titles must differ.
      let title = label(t).replace(/[\u0000-\u001f\u007f]/g, " ").trim().slice(0, 24) || `字幕轨 ${i + 1}`;
      if (used.has(title) || RESERVED.has(title)) title = `${title} (${i + 1})`;
      used.add(title);
      return title;
    });
  }
  function syncTrackMenu(s: SubtitlesState) {
    const titles = menuTitles(s.tracks);
    const key = JSON.stringify(s.tracks.slice(0, MENU_TRACKS).map((t, i) => [t.id, titles[i]]));
    if (key === trackKey) return;
    trackKey = key;
    const tracks = s.tracks.slice(0, MENU_TRACKS);
    rebuilding = rebuilding.then(async () => {
      await trackMenu.dispose();
      trackMenu = plugin.scope();
      tracks.forEach((track, i) =>
        trackMenu.commands.add({
          id: `track-${i + 1}`, path: ["字幕", "选择字幕轨", titles[i]!], order: 303 + i,
          group: "subtitle-track", radio: true, ...idle,
          checked: () => track.id === subtitles.state?.selected?.id,
          run: () => busy || announce("正在切换字幕轨…", select(track, s.sourceId)),
        }),
      );
    });
  }
  plugin.commands.add({
    id: "track-cycle", path: ["字幕", "选择字幕轨", "依次切换"], order: 311, group: "subtitle-cycle", bindings: ["Alt+L"],
    enabled: () => !!subtitles.state?.tracks.length,
    reason: () => (subtitles.state?.tracks.length ? null : "没有字幕轨"),
    run: cycle,
  });

  // 字幕样式 ▶ (D06, D07): same edits as the 字幕微调 buttons, reported on screen.
  /** Key-driven style edits: never blocked by a running operation, so held keys keep repeating. */
  const styleKey = (scope: Scope, id: string, path: string[], order: number, group: string,
    binding: string, apply: () => SubtitleStyle | null, text: (style: SubtitleStyle) => string, repeat = true) =>
    scope.commands.add({
      id, path, order, group, bindings: [binding], repeat,
      run: () => {
        const style = apply();
        say(style ? text(style) : status);
        plugin.update();
      },
    });
  styleKey(plugin, "size-up", ["字幕", "字幕样式", "字号 +"], 325, "style-font", "Alt+PageUp", () => sizeBy(1),
    (s) => `字幕尺寸：增加(${s.fontSize})`);
  styleKey(plugin, "size-down", ["字幕", "字幕样式", "字号 −"], 326, "style-font", "Alt+PageDown", () => sizeBy(-1),
    (s) => `字幕尺寸：减小(${s.fontSize})`);
  styleKey(plugin, "move-up", ["字幕", "字幕样式", "上移"], 330, "style-move", "Alt+ArrowUp", () => moveBy(1),
    (s) => `字幕位置：上移(${s.marginPercent}%)`);
  styleKey(plugin, "move-down", ["字幕", "字幕样式", "下移"], 331, "style-move", "Alt+ArrowDown", () => moveBy(-1),
    (s) => `字幕位置：下移(${s.marginPercent}%)`);
  styleKey(plugin, "move-reset", ["字幕", "字幕样式", "复位位置"], 334, "style-move", "Alt+Home",
    () => styleChange(() => ({ marginPercent: BASE.marginPercent })), (s) => `字幕位置：默认(${s.marginPercent}%)`, false);

  // 同步 ▶: the 超前 / 滞后 titles show the 同步单位, so they live in a child scope rebuilt when it changes.
  let syncMenu = plugin.scope();
  let syncMenuStep = 0;
  let syncRebuild = Promise.resolve();
  const offsetText = (s: SubtitleStyle) => syncText(s.offsetMs);
  function syncSyncMenu() {
    if (syncMenuStep === syncStep) return;
    const step = (syncMenuStep = syncStep);
    syncRebuild = syncRebuild.then(async () => {
      await syncMenu.dispose();
      syncMenu = plugin.scope();
      styleKey(syncMenu, "sync-earlier", ["字幕", "同步", `超前 ${stepSeconds(step)} 秒`], 339, "subtitle-sync", ".",
        () => syncBy(-1), offsetText);
      styleKey(syncMenu, "sync-later", ["字幕", "同步", `滞后 ${stepSeconds(step)} 秒`], 340, "subtitle-sync", ",",
        () => syncBy(1), offsetText);
    });
  }
  syncSyncMenu();
  styleKey(plugin, "sync-reset", ["字幕", "同步", "复位"], 341, "subtitle-sync", "/", syncReset, offsetText);
  plugin.commands.add({
    id: "sync-input", path: ["字幕", "同步", "手动调整同步…"], order: 342, group: "subtitle-sync-input", run: showSync,
  });
  const tool = (id: string, title: string, order: number, run: () => unknown, extra: Partial<CommandOptions> = {}) =>
    plugin.commands.add({ id, path: ["字幕", title], order, group: "subtitle-tools", run, ...extra });
  tool("reload", "重载字幕", 370, () => busy || announce("正在重载字幕…", reload()), { bindings: ["Ctrl+Alt+Y"], ...idle });
  tool("refresh", "刷新字幕轨", 371, () => busy || announce("正在刷新…", refresh));
  tool("tune", "字幕微调…", 372, showTune);
  tool("settings", "字幕设置…", 373, showSettings);

  // A new source cancels pending work; a new source or selection drops the unconfirmed style.
  subtitles.onChange((s) => {
    if (seen && s.sourceId > seen.sourceId) {
      cancel();
      status = "媒体已切换，已取消进行中的操作。";
    }
    if (!seen || s.sourceId !== seen.sourceId || s.selected?.id !== seen.selected?.id) desired = undefined;
    if (s.selected) lastShown = s.selected.id;
    // Loaded tracks belong to their source; forget the ones that are gone.
    for (const id of [...loaded.keys()]) if (!s.tracks.some((t) => t.id === id)) forget(id);
    seen = s;
    syncTrackMenu(s);
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
  await rebuilding;
  await syncRebuild;
}

function settingsFile(plugin: Plugin) {
  try {
    return join(plugin.dataDirectory, "settings.json");
  } catch {
    return null; // no data directory: settings last for this run only
  }
}
/** A missing or broken file reads as the defaults. */
function readJson(file: string | null): unknown {
  if (!file) return null;
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}
