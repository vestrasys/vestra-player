import type { PlaybackStats, PlayerState, Plugin } from "@vestra-player/sdk";
import { aboutWindow } from "./about.ts";
import { propertiesWindow } from "./dialog.ts";

/** "mm:ss", or "h:mm:ss" from one hour. */
function clock(seconds: number | null) {
  if (seconds === null || !Number.isFinite(seconds)) return "--:--";
  const total = Math.max(0, Math.floor(seconds));
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

const or = (value: unknown) => (value === null || value === undefined || value === "" ? "-" : String(value));

/** The file name of a path, or the last path segment of a URL (its host when it has none). */
export function fileName(source: string | null) {
  if (!source) return "未打开媒体";
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(source)) {
    try {
      const url = new URL(source);
      const last = url.pathname.split("/").filter(Boolean).at(-1);
      return last ? decodeURIComponent(last) : url.hostname || source;
    } catch {
      return source;
    }
  }
  return source.split(/[\\/]/).filter(Boolean).at(-1) ?? source;
}

/** The overlay text (ui-alignment §9.2): one fact per line, Chinese labels. */
export function describe(state: PlayerState, stats: PlaybackStats) {
  const { video, audio } = stats;
  const size = video?.width && video?.height ? `${video.width}×${video.height}` : "-";
  return [
    `文件：${fileName(state.source)}`,
    `时间：${clock(state.position)} / ${clock(state.duration)}`,
    `视频：${or(video?.codec)} ${size}`,
    `帧率：${video?.fps ? `${video.fps.toFixed(2)} fps` : "-"}`,
    `音频：${or(audio?.codec)} ${audio?.sampleRate ? `${audio.sampleRate} Hz` : "-"} ${audio?.channels ? `${audio.channels} 声道` : "-"}`,
    `后端：${or(stats.backend)}（会话 ${stats.session}）`,
    `已显示帧：${stats.framesShown}`,
    `丢帧：${or(stats.framesDropped)}`,
  ].join("\n");
}

/** Short codec names: mpv reports long descriptions such as "H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10". */
const CODECS: [RegExp, string][] = [
  [/\b(h\.?264|avc1?)\b/i, "H.264"],
  [/\b(hevc|h\.?265)\b/i, "HEVC"],
  [/\bav1\b/i, "AV1"],
  [/\bvp9\b/i, "VP9"],
  [/\bvp8\b/i, "VP8"],
  [/\bprores\b/i, "ProRes"],
  [/\b(vc-?1|wmv3)\b/i, "VC-1"],
  [/\bmpeg-?4\b|\bmp4v\b/i, "MPEG-4"],
  [/\bmpeg-?2\b|\bmpeg2video\b/i, "MPEG-2"],
  [/\bmpeg-?1\b|\bmpeg1video\b/i, "MPEG-1"],
  [/\btheora\b/i, "Theora"],
  [/\be-?ac-?3\b/i, "E-AC-3"],
  [/\bac-?3\b/i, "AC-3"],
  [/\btruehd\b/i, "TrueHD"],
  [/\bdts\b/i, "DTS"],
  [/\baac\b/i, "AAC"],
  [/\bmp3\b|layer 3/i, "MP3"],
  [/\bopus\b/i, "Opus"],
  [/\bvorbis\b/i, "Vorbis"],
  [/\bflac\b/i, "FLAC"],
  [/\balac\b/i, "ALAC"],
  [/\bpcm\b/i, "PCM"],
];
export function shortCodec(codec: string | null | undefined) {
  if (!codec) return null;
  for (const [pattern, name] of CODECS) if (pattern.test(codec)) return name;
  // Unknown: the first part of the description, at most 12 characters.
  const first = codec.split(/\s+\/\s+|\s*\(/)[0]!.trim();
  return first.length > 12 ? `${first.slice(0, 11)}…` : first || null;
}

/** I03 lines must fit the narrowest video area (318 px of 11 px text): 44 columns, CJK counting two. */
export const BRIEF_COLUMNS = 44;
/**
 * IA-INFO-01: the columns the info block has in a video area `width` points
 * wide (10 px margins, 6 px padding, about 6.5 px a column); never fewer than
 * `BRIEF_COLUMNS` (headless players report 0).
 */
export const briefColumns = (width: number) => Math.max(BRIEF_COLUMNS, Math.floor((width - 32) / 6.5));
const wide = (char: string) => /[ᄀ-ᅟ⺀-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]|[\u{20000}-\u{3fffd}]/u.test(char);
/** Cut a line to `columns` display columns, ending in "…" when cut. */
export function fit(line: string, columns = BRIEF_COLUMNS) {
  const chars = [...line];
  let width = 0;
  for (let i = 0; i < chars.length; i++) {
    width += wide(chars[i]!) ? 2 : 1;
    if (width > columns) {
      let kept = chars.slice(0, i);
      while (kept.length && kept.reduce((sum, c) => sum + (wide(c) ? 2 : 1), 0) > columns - 1) kept = kept.slice(0, -1);
      return `${kept.join("")}…`;
    }
  }
  return line;
}

/** What the short information adds from other public state (IA-INFO-01); `null` / absent = left out. */
export interface BriefExtras {
  /** The current chapter's title (`第 N 章` when it has none). */
  chapter?: string | null;
  /** The selected subtitle track's name. */
  subtitle?: string | null;
}

/** IA-INFO-01: the chapter playing at `position`: the last one starting at or before it. */
export function currentChapter(chapters: readonly { title: string | null; start: number }[], position: number | null) {
  if (position === null || !Number.isFinite(position)) return null;
  let found = -1;
  chapters.forEach((chapter, index) => {
    if (chapter.start <= position + 1e-3) found = index;
  });
  if (found < 0) return null;
  return chapters[found]!.title?.trim() || `第 ${found + 1} 章`;
}

/** IA-INFO-01: the selected subtitle track's title, else its language, else 「字幕」; `null` when none is selected. */
export function subtitleName(track: { title: string; language: string | null } | null | undefined) {
  if (!track) return null;
  return track.title.trim() || track.language?.trim() || "字幕";
}

/**
 * I03 / IA-INFO-01 short information, up to three lines: the file name, then
 * `01:23 / 45:00 · 第二章 · H.264 1920×1080 · AAC 2 声道`, then the selected
 * subtitle track's name; unknown items (and the subtitle line without a
 * selected track) are left out, codecs use short names, and each line fits
 * the video area (cut with "…").
 */
export function brief(state: PlayerState, stats: PlaybackStats | null, extras: BriefExtras = {}, columns = BRIEF_COLUMNS) {
  if (!state.source) return "未打开媒体";
  const { video, audio } = stats ?? { video: null, audio: null };
  const join = (...parts: (string | null | undefined | false)[]) => parts.filter(Boolean).join(" ");
  const items = [
    state.position !== null && Number.isFinite(state.position)
      ? `${clock(state.position)} / ${state.duration === null ? "直播" : clock(state.duration)}`
      : null,
    extras.chapter,
    join(shortCodec(video?.codec), !!video?.width && !!video?.height && `${video.width}×${video.height}`),
    join(shortCodec(audio?.codec), !!audio?.channels && `${audio.channels} 声道`),
  ].filter(Boolean);
  const lines = [fit(fileName(state.source), columns)];
  if (items.length) lines.push(fit(items.join(" · "), columns));
  if (extras.subtitle) lines.push(fit(extras.subtitle, columns));
  return lines.join("\n");
}

const BRIEF_MS = 3000;

/**
 * Playback information: Tab toggles the full overlay; Shift+Tab shows the
 * short information for three seconds. Both share this plugin's one panel,
 * short first, a blank line between; each refreshes every second while shown.
 * Ctrl+F1 opens 播放属性 (I05) and F1 关于 Vestra Player (I09), tool windows.
 */
export default async function activate(plugin: Plugin) {
  const panel = plugin.osd.panel();
  let shown = false;
  let briefOn = false;
  let stats: PlaybackStats | null = null; // the last successful read
  let timer: ReturnType<typeof setInterval> | undefined;
  let briefTimer: ReturnType<typeof setTimeout> | undefined;
  /** IA-INFO-01: the current source's chapters (read once per source) for the short information. */
  let chapters: { sourceId: number; list: readonly { title: string | null; start: number }[] } | null = null;
  const briefShown = () => briefOn;
  const extras = (): BriefExtras => {
    const state = plugin.player.state;
    return {
      chapter: chapters?.sourceId === state.sourceId ? currentChapter(chapters.list, state.position) : null,
      subtitle: state.source ? subtitleName(plugin.subtitles.state?.selected) : null,
    };
  };
  /** Write the panel from the last stats; nothing shown hides it. */
  const draw = () => {
    const blocks = [
      briefShown() ? brief(plugin.player.state, stats, extras(), briefColumns(plugin.window.state.videoArea.width)) : null,
      shown && stats ? describe(plugin.player.state, stats) : null,
    ].filter((block): block is string => block !== null);
    panel.set(blocks.length ? blocks.join("\n\n") : null);
  };
  /** The chapter list (once per source) and the subtitle selection (no event exists: read each time). */
  const readExtras = async () => {
    const sourceId = plugin.player.state.sourceId;
    await Promise.all([
      chapters?.sourceId === sourceId
        ? null
        : plugin.player.chapters().then((read) => {
            // A stale reply (the source changed meanwhile) is dropped; the next second reads again.
            if (read.sourceId === plugin.player.state.sourceId) chapters = { sourceId: read.sourceId, list: read.chapters };
          }, () => {}),
      plugin.subtitles.refresh().catch(() => {}),
    ]);
  };
  const render = async () => {
    const [read] = await Promise.all([plugin.player.stats(), briefShown() ? readExtras() : null]);
    stats = read;
    if (shown || briefShown()) draw(); // may have been switched off meanwhile
  };
  const refresh = () => void render().catch(() => {}); // keep the last text; try again next second
  /** One 1-second refresh while anything is shown. */
  const tick = () => {
    if ((shown || briefShown()) && !timer) timer = setInterval(refresh, 1000);
    if (!shown && !briefShown()) {
      clearInterval(timer);
      timer = undefined;
    }
  };
  plugin.commands.add({
    id: "info.toggle",
    path: ["播放信息"],
    order: 810,
    group: "windows",
    bindings: ["Tab"],
    checked: () => shown,
    run: () => {
      shown = !shown;
      tick();
      if (!shown) return draw();
      refresh();
    },
  });
  plugin.commands.add({
    id: "info.brief",
    title: "简要信息",
    bindings: ["Shift+Tab"],
    run: () => {
      // Each press restarts the three seconds.
      briefOn = true;
      clearTimeout(briefTimer);
      briefTimer = setTimeout(() => {
        briefOn = false;
        tick();
        draw();
      }, BRIEF_MS);
      tick();
      if (shown && stats) draw(); // the overlay's stats are at most a second old: show at once
      refresh();
    },
  });
  const showProperties = propertiesWindow(plugin, (text) => void plugin.osd.show(text).catch(() => {}));
  plugin.commands.add({
    id: "info.properties",
    path: ["播放属性…"],
    order: 811,
    group: "windows",
    bindings: ["Ctrl+F1"],
    run: showProperties,
  });
  plugin.commands.add({
    id: "info.about",
    path: ["关于 Vestra Player…"],
    order: 880,
    group: "windows",
    bindings: ["F1"],
    run: aboutWindow(plugin, (text) => void plugin.osd.show(text).catch(() => {})),
  });
  plugin.onDispose(() => {
    clearInterval(timer);
    clearTimeout(briefTimer);
  });
}
