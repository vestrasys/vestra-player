import type {
  Plugin,
  SubtitleStyle,
  SubtitleTrack,
  SubtitlesState,
} from "../../sdk/plugin/index.ts";
import { readSrt } from "./srt.ts";

const COLORS = ["white", "yellow", "cyan"] as const;
const CANCELLED = "操作已取消或媒体已切换；请刷新当前字幕状态。";
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
const label = (t: SubtitleTrack) =>
  `${t.title || (t.kind === "embedded" ? `内嵌 ${t.embeddedId}` : "外部 SRT")}${t.language ? ` · ${t.language}` : ""}`;

type Action = (ticket: number) => Promise<void>;

/** Subtitle workbench: one window, four menu entries, serialized subtitle operations. */
export default async function activate(plugin: Plugin) {
  const { subtitles } = plugin;
  const window = plugin.windows.create("ui/workbench.slint", { id: "subtitle-workbench" });
  let status = "选择字幕轨或加载 UTF-8 SRT。";
  let busy = false;
  let epoch = 0; // bumped by cancel and media switches; stale actions stop at the next guard
  let aborter = new AbortController(); // aborted together with each epoch bump (stops an upload)
  let chain = Promise.resolve();
  let desired: SubtitleStyle | undefined; // edited style shown before the host confirms it
  let editing: SubtitlesState | null = null; // selection the pending style edit belongs to
  let styleTimer: ReturnType<typeof setTimeout> | undefined;
  let seen: SubtitlesState | null = null;

  const guard = (ticket: number, source?: number) => {
    if (epoch !== ticket || (source !== undefined && source !== subtitles.state?.sourceId))
      throw new Error(CANCELLED);
  };
  /** One operation at a time; the menu shows busy meanwhile. */
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
      } finally {
        busy = false;
        if (!subtitles.state) await subtitles.refresh().catch(() => {});
        plugin.update(); // also runs from the style timer, outside any SDK handler
      }
    });
    return chain;
  }
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
    status = "已请求取消。已交给宿主的操作可能已生效，请刷新确认；系统选择器请在其窗口取消。";
  }

  const refresh: Action = async () => {
    await subtitles.refresh();
    desired = undefined;
    status = "字幕状态已刷新。";
  };
  const select =
    (track: SubtitleTrack | null, source?: number): Action =>
    async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source ?? s.sourceId);
      await subtitles.select(track, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = track ? "字幕轨已选择，请查看宿主确认状态。" : "字幕已关闭。";
    };
  const load: Action = async (ticket) => {
    const signal = aborter.signal;
    const initial = await subtitles.refresh();
    guard(ticket, initial.sourceId);
    const path = await plugin.dialogs.openFile({ title: "加载 UTF-8 SRT 字幕" });
    guard(ticket, initial.sourceId);
    if (!path) return void (status = "已取消选择字幕文件。");
    const bytes = await readSrt(path);
    guard(ticket, initial.sourceId);
    const s = await subtitles.refresh();
    guard(ticket, initial.sourceId);
    guard(ticket, s.sourceId);
    try {
      await subtitles.load(bytes, { ifCurrent: s, signal }); // uploads, commits and selects
    } catch (e) {
      if ((e as Error).name === "AbortError") throw new Error(CANCELLED); // discarded, never committed
      throw e;
    }
    guard(ticket, s.sourceId);
    desired = undefined;
    status = `SRT 已加载（${bytes.length} 字节），轨道：${subtitles.state?.trackConfirmation}。`;
  };

  /** Slider edits merge for 100 ms and apply only to the track selected when the edit began. */
  function styleChange<K extends keyof SubtitleStyle>(key: K, value: SubtitleStyle[K]) {
    const s = subtitles.state;
    if (!s || !s.selected?.styleSupported || !s.fonts[0]) return;
    const base = { fontId: s.fonts[0].id, fontSize: 36, color: "white", outline: 2, marginPercent: 5, offsetMs: 0 } as const;
    desired = { ...(desired ?? s.style ?? base), [key]: value };
    editing ??= s;
    const requested = { ...desired }, pinned = editing;
    clearTimeout(styleTimer);
    styleTimer = setTimeout(() => {
      stopStyleTimer();
      void run("正在应用字幕样式…", async (ticket) => {
        guard(ticket, pinned.sourceId);
        try {
          await subtitles.setStyle(requested, { ifCurrent: pinned });
        } catch (e) {
          if ((e as { code?: string }).code === "stale") throw new Error("字幕轨已变化，请在当前轨道重新调整。");
          throw e;
        }
        guard(ticket, pinned.sourceId);
        status = `字幕样式：${subtitles.state?.styleConfirmation}。`;
      });
    }, 100);
  }

  window.bind(() => {
    const s = subtitles.state, selected = s?.selected ?? null;
    const style = desired ?? s?.style ?? undefined;
    return {
      statusText: status,
      sourceText: s ? `媒体版本 ${s.sourceId} · ${s.phase} · ${s.tracks.length} 条字幕轨` : "尚无字幕状态",
      confirmationText: s ? `轨道：${s.trackConfirmation} · 样式：${s.styleConfirmation}${s.error ? ` · ${s.error}` : ""}` : "",
      trackLabels: ["关闭字幕", ...(s?.tracks.map(label) ?? [])],
      selectedTrack: selected && s ? s.tracks.findIndex((t) => t.id === selected.id) + 1 : 0,
      busy,
      canStyle: !!selected?.styleSupported,
      canRemove: !!selected?.owned,
      styleReason: selected
        ? selected.styleSupported ? "样式按 1080p 参考高度缩放。" : (selected.styleReason ?? "此字幕轨不支持样式。")
        : "选择字幕后可调整样式与延迟。",
      fontLabels: s?.fonts.map((f) => f.family) ?? [],
      selectedFont: Math.max(0, s?.fonts.findIndex((f) => f.id === style?.fontId) ?? 0),
      delayMs: style?.offsetMs ?? 0,
      fontSize: style?.fontSize ?? 36,
      outlineSize: style?.outline ?? 2,
      marginPercent: style?.marginPercent ?? 5,
      selectedColor: Math.max(0, COLORS.indexOf(style?.color ?? "white")),
    };
  });

  window.on("cancel", cancel);
  window.on("loadFile", () => busy || run("请选择 SRT…", load));
  window.on("refresh", () => busy || run("正在刷新…", refresh));
  window.on("selectTrack", (index: number) => {
    const s = subtitles.state;
    if (busy || !Number.isInteger(index)) return;
    const track = index === 0 ? null : s?.tracks[index - 1];
    if (track !== undefined) return run("正在切换字幕轨…", select(track, s?.sourceId));
  });
  window.on("removeSelected", () => {
    const track = subtitles.state?.selected, source = subtitles.state?.sourceId;
    if (busy || !track?.owned) return;
    return run("正在移除字幕…", async (ticket) => {
      const s = await subtitles.refresh();
      guard(ticket, source);
      if (s.selected?.id !== track.id) throw new Error("选中的字幕已变化，请重新选择。");
      await subtitles.remove(track, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = "外部字幕已移除。";
    });
  });
  window.on("resetStyle", () => {
    if (busy) return;
    stopStyleTimer();
    return run("正在撤销本插件样式…", async (ticket) => {
      const s = await subtitles.refresh();
      await subtitles.setStyle(null, { ifCurrent: s });
      guard(ticket, s.sourceId);
      desired = undefined;
      status = "本插件的样式与延迟已撤销。";
    });
  });
  const slider = (name: string, apply: (value: number) => void) =>
    window.on(name, (value: number) => Number.isFinite(value) && apply(value));
  const clamp = (value: number, min: number, max: number) => Math.round(Math.max(min, Math.min(max, value)));
  slider("changeDelay", (v) => styleChange("offsetMs", clamp(v, -10000, 10000)));
  // Ranges are the host's accepted style limits; anything wider fails as `invalid`.
  slider("changeSize", (v) => styleChange("fontSize", clamp(v, 16, 72)));
  slider("changeOutline", (v) => styleChange("outline", clamp(v, 0, 4)));
  slider("changeMargin", (v) => styleChange("marginPercent", clamp(v, 2, 15)));
  slider("changeFont", (v) => {
    const font = Number.isInteger(v) ? subtitles.state?.fonts[v] : undefined;
    if (font) styleChange("fontId", font.id);
  });
  slider("changeColor", (v) => {
    const color = Number.isInteger(v) ? COLORS[v] : undefined;
    if (color) styleChange("color", color);
  });
  plugin.onError((e) => (status = e.message));

  const menu = (id: string, title: string, order: number, text: string, action: Action) =>
    plugin.commands.add({
      id,
      path: ["字幕", title],
      order,
      enabled: () => !busy,
      busy: () => busy,
      reason: () => (busy ? "字幕操作进行中" : null),
      run: () => busy || run(text, action),
    });
  menu("workbench", "字幕工作台…", 444, "正在打开字幕工作台…", async () => {
    await subtitles.refresh();
    await window.show();
    status = "字幕工作台已就绪。";
  });
  menu("load", "加载外部 SRT…", 443, "请选择 SRT…", async (ticket) => {
    await window.show();
    await load(ticket);
  });
  menu("disable", "关闭字幕", 441, "正在关闭字幕…", select(null));
  menu("refresh", "刷新字幕轨", 442, "正在刷新…", refresh);

  // A new source cancels pending work; a new source or selection drops the unconfirmed style.
  subtitles.onChange((s) => {
    if (seen && s.sourceId > seen.sourceId) {
      cancel();
      status = "媒体已切换，正在刷新字幕轨…";
    }
    if (!seen || s.sourceId !== seen.sourceId || s.selected?.id !== seen.selected?.id) desired = undefined;
    seen = s;
  });
  // The host has no subtitle event; the open workbench also re-reads every 1.5 s.
  const poll = setInterval(() => {
    if (window.opened && !busy)
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
  await subtitles.refresh();
}
