import { basename } from "node:path";
import type {
  Plugin,
  SubtitleStyle,
  SubtitleTrack,
  SubtitlesState,
} from "../../sdk/plugin/index.ts";
import { readSrt } from "./srt.ts";

const COLORS = ["white", "yellow", "cyan"] as const;
/** 字幕 ▶ 选择字幕轨 lists at most this many tracks (orders 303–310); the rest are in 字幕微调. */
const MENU_TRACKS = 8;
const SYNC_STEP_MS = 500;
const SIZE_STEP = 2;
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

/** Subtitles: 字幕微调 and 字幕设置 windows, 字幕 ▶ menu, serialized subtitle operations. */
export default async function activate(plugin: Plugin) {
  const { subtitles, osd } = plugin;
  const tune = plugin.windows.create("ui/tune.slint", { id: "subtitle-tune" });
  const settings = plugin.windows.create("ui/settings.slint", { id: "subtitle-settings" });
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
    await subtitles.load(bytes, { ifCurrent: s, signal }); // uploads, commits and selects
    guard(ticket, s.sourceId);
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
    styleChange((style) => ({ offsetMs: clamp(style.offsetMs + steps * SYNC_STEP_MS, -10000, 10000) }));
  const syncReset = () => styleChange(() => ({ offsetMs: 0 }));
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
  settings.bind(() => {
    const s = subtitles.state, style = current();
    return {
      ...common(),
      fontLabels: s?.fonts.map((f) => f.family) ?? [],
      selectedFont: Math.max(0, s?.fonts.findIndex((f) => f.id === style.fontId) ?? 0),
      selectedColor: Math.max(0, COLORS.indexOf(style.color)),
      outlineSize: style.outline,
      marginPercent: style.marginPercent,
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
  tune.on("sizeStep", number((v) => styleChange((s) => ({ fontSize: clamp(s.fontSize + Math.sign(v) * SIZE_STEP, 16, 72) }))));
  tune.on("positionStep", number((v) => styleChange((s) => ({ marginPercent: clamp(s.marginPercent + Math.sign(v), 2, 15) }))));
  tune.on("syncStep", number((v) => syncBy(Math.sign(v))));
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
      if (used.has(title)) title = `${title} (${i + 1})`;
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
  const sync = (id: string, title: string, order: number, binding: string, apply: () => SubtitleStyle | null) =>
    plugin.commands.add({
      id, path: ["字幕", "同步", title], order, group: "subtitle-sync", bindings: [binding], repeat: true, ...idle,
      run: () => {
        const style = apply();
        say(style ? syncText(style.offsetMs) : status);
        plugin.update();
      },
    });
  sync("sync-earlier", "超前 0.5 秒", 311, ">", () => syncBy(-1));
  sync("sync-later", "滞后 0.5 秒", 312, "<", () => syncBy(1));
  sync("sync-reset", "复位", 313, "/", syncReset);
  const tool = (id: string, title: string, order: number, run: () => unknown) =>
    plugin.commands.add({ id, path: ["字幕", title], order, group: "subtitle-tools", run });
  tool("refresh", "刷新字幕轨", 314, () => busy || announce("正在刷新…", refresh));
  tool("tune", "字幕微调…", 315, showTune);
  tool("settings", "字幕设置…", 316, showSettings);

  // A new source cancels pending work; a new source or selection drops the unconfirmed style.
  subtitles.onChange((s) => {
    if (seen && s.sourceId > seen.sourceId) {
      cancel();
      status = "媒体已切换，已取消进行中的操作。";
    }
    if (!seen || s.sourceId !== seen.sourceId || s.selected?.id !== seen.selected?.id) desired = undefined;
    if (s.selected) lastShown = s.selected.id;
    seen = s;
    syncTrackMenu(s);
  });
  // The host has no subtitle event; an open window also re-reads every 1.5 s.
  const poll = setInterval(() => {
    if ((tune.opened || settings.opened) && !busy)
      subtitles.refresh().catch((e) => {
        status = errorText(e);
        plugin.update();
      });
  }, 1500);
  poll.unref();
  plugin.onDispose(() => {
    clearInterval(poll);
    stopStyleTimer();
  });
  syncTrackMenu(await subtitles.refresh());
  await rebuilding;
}
