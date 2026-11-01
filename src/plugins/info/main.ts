import type { I18n, PlaybackStats, PlayerState, Plugin, TextArg, TextList, TextValue } from "@vestra-player/sdk";
import { aboutWindow } from "./about.ts";
import { propertiesWindow } from "./dialog.ts";
import { Updater } from "./updates.ts";

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
export function fileName(source: string) {
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

/** `"hardware (d3d11va-copy)"` / `"software"` from `stats().video.decoding`; `null` while unknown. */
export function decoding(i18n: I18n, video: PlaybackStats["video"]): TextArg | null {
  // TRANSLATORS: how the video is decoded, after "Decoder:": by the CPU.
  if (video?.decoding === "software") return i18n.text("software");
  if (video?.decoding !== "hardware") return null;
  if (video.hwdec && video.hwdec !== "no") {
    // TRANSLATORS: how the video is decoded, after "Decoder:": by the GPU; {} is the method, e.g. d3d11va-copy.
    return i18n.text("hardware ({})", [video.hwdec]);
  }
  // TRANSLATORS: how the video is decoded, after "Decoder:": by the GPU.
  return i18n.text("hardware");
}

/** `30 fps (source 60)`: new frames the window showed per second, and the source's rate; `null` while not measured. */
export function displayed(i18n: I18n, stats: PlaybackStats): TextArg | null {
  if (stats.presentedFps === null || stats.presentedFps === undefined || !stats.video) return null;
  const shown = Number(stats.presentedFps.toFixed(1));
  const source = stats.video.fps ? Number(stats.video.fps.toFixed(2)) : null;
  // TRANSLATORS: frames per second the window showed; "source" is the video's own frame rate.
  return source === null ? i18n.text("{} fps", [shown]) : i18n.text("{} fps (source {})", [shown, source]);
}

/** The overlay text (ui-spec §9.2): one fact per line, as text values the player translates. */
export function describe(i18n: I18n, state: PlayerState, stats: PlaybackStats): TextList {
  const { text, list } = i18n;
  const { video, audio } = stats;
  const size = video?.width && video?.height ? `${video.width}×${video.height}` : "-";
  return list([
    text("File: {}", [state.source ? fileName(state.source) : text("No media open")]),
    text("Time: {} / {}", [clock(state.position), clock(state.duration)]),
    text("Video: {} {}", [or(video?.codec), size]),
    text("Decoder: {}", [decoding(i18n, video) ?? "-"]),
    text("Frame rate: {}", [video?.fps ? `${video.fps.toFixed(2)} fps` : "-"]),
    // TRANSLATORS: the frame rate that really reached the screen, e.g. "30 fps (source 60)".
    text("Displayed: {}", [displayed(i18n, stats) ?? "-"]),
    text("Audio: {} {} {}", [or(audio?.codec), audio?.sampleRate ? `${audio.sampleRate} Hz` : "-", audio?.channels ? text("{} channels", [audio.channels]) : "-"]),
    text("Backend: {} (session {})", [or(stats.backend), stats.session]),
    text("Frames shown: {}", [stats.framesShown]),
    text("Dropped frames: {}", [or(stats.framesDropped)]),
  ], "lines");
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
 * `BRIEF_COLUMNS` (headless players report 0), at most the 1000 `maxColumns` takes.
 */
export const briefColumns = (width: number) => Math.min(1000, Math.max(BRIEF_COLUMNS, Math.floor((width - 32) / 6.5)));

/** What the short information adds from other public state (IA-INFO-01); `null` / absent = left out. */
export interface BriefExtras {
  /** The current chapter's title (`Chapter N` when it has none). */
  chapter?: TextArg | null;
  /** The selected subtitle track's name. */
  subtitle?: TextArg | null;
}

/** IA-INFO-01: the chapter playing at `position`: the last one starting at or before it. */
export function currentChapter(i18n: I18n, chapters: readonly { title: string | null; start: number }[], position: number | null): TextArg | null {
  if (position === null || !Number.isFinite(position)) return null;
  let found = -1;
  chapters.forEach((chapter, index) => {
    if (chapter.start <= position + 1e-3) found = index;
  });
  if (found < 0) return null;
  return chapters[found]!.title?.trim() || i18n.text("Chapter {}", [found + 1]);
}

/** IA-INFO-01: the selected subtitle track's title, else its language, else "Subtitles"; `null` when none is selected. */
export function subtitleName(i18n: I18n, track: { title: string; language: string | null } | null | undefined): TextArg | null {
  if (!track) return null;
  return track.title.trim() || track.language?.trim() || i18n.text("Subtitles");
}

/**
 * I03 / IA-INFO-01 short information, up to four lines: the file name, then
 * `01:23 / 45:00 · Part Two · H.264 1920×1080 · AAC 2 ch`, then
 * `Decoder: hardware (videotoolbox) · Displayed: 30 fps (source 60)`, then the
 * selected subtitle track's name; unknown items (and the subtitle line without
 * a selected track) are left out, and codecs use short names. The player cuts
 * each line to the video area after translating it (`maxColumns`).
 */
export function brief(i18n: I18n, state: PlayerState, stats: PlaybackStats | null, extras: BriefExtras = {}): TextValue {
  const { text, list } = i18n;
  if (!state.source) return text("No media open");
  const { video, audio } = stats ?? { video: null, audio: null };
  const join = (...parts: (string | null | undefined | false)[]) => parts.filter(Boolean).join(" ");
  const channels = audio?.channels ? text("{} ch", [audio.channels]) : null;
  const codec = shortCodec(audio?.codec);
  const items = [
    state.position !== null && Number.isFinite(state.position)
      ? text("{} / {}", [clock(state.position), state.duration === null ? text("Live") : clock(state.duration)])
      : null,
    extras.chapter,
    join(shortCodec(video?.codec), !!video?.width && !!video?.height && `${video.width}×${video.height}`),
    codec && channels ? text("{} {}", [codec, channels]) : (codec ?? channels),
  ].filter((item): item is TextArg => !!item);
  const lines: TextArg[] = [fileName(state.source)];
  if (items.length) lines.push(items.reduce((line, item) => text("{} · {}", [line, item])));
  const decoder = stats ? decoding(i18n, stats.video) : null;
  const shown = stats ? displayed(i18n, stats) : null;
  const frames: TextArg[] = [];
  if (decoder !== null) frames.push(text("Decoder: {}", [decoder]));
  if (shown !== null) frames.push(text("Displayed: {}", [shown]));
  if (frames.length) lines.push(frames.reduce((line, item) => text("{} · {}", [line, item])));
  if (extras.subtitle) lines.push(extras.subtitle);
  return list(lines, "lines");
}

const BRIEF_MS = 3000;

/**
 * Playback information: Tab toggles the full overlay; Shift+Tab shows the
 * short information for three seconds. Both share this plugin's one panel,
 * short first, a blank line between; each refreshes every second while shown.
 * Ctrl+F1 opens Playback Properties (I05) and F1 About Vestra Player (I09), tool windows.
 * Check for Updates… (J10) opens About and checks the player's releases; the
 * automatic check (Options ▸ General ▸ Updates) runs at most once a day.
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
      chapter: chapters?.sourceId === state.sourceId ? currentChapter(plugin.i18n, chapters.list, state.position) : null,
      subtitle: state.source ? subtitleName(plugin.i18n, plugin.subtitles.state?.selected) : null,
    };
  };
  /** Write the panel from the last stats; nothing shown hides it. */
  const draw = () => {
    const blocks = [
      briefShown() ? brief(plugin.i18n, plugin.player.state, stats, extras()) : null,
      shown && stats ? describe(plugin.i18n, plugin.player.state, stats) : null,
    ].filter((block) => block !== null);
    // A blank line between the blocks; the short information's lines fit the video area.
    const columns = briefShown() ? { maxColumns: briefColumns(plugin.window.state.videoArea.width) } : {};
    panel.set(blocks.length ? plugin.i18n.list(blocks.length > 1 ? [blocks[0]!, "", blocks[1]!] : blocks, "lines") : null, null, columns);
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
    path: ["Playback Info"],
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
    title: "Brief Info",
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
  const showProperties = propertiesWindow(plugin);
  plugin.commands.add({
    id: "info.properties",
    path: ["Playback Properties…"],
    order: 811,
    group: "windows",
    bindings: ["Ctrl+F1"],
    run: showProperties,
  });
  const updater = new Updater(plugin);
  const showAbout = aboutWindow(plugin, updater);
  plugin.commands.add({
    id: "info.about",
    path: ["About Vestra Player…"],
    order: 880,
    group: "windows",
    bindings: ["F1"],
    run: showAbout,
  });
  // J10: right after About; the About window shows the result and the actions.
  updater.command = plugin.commands.add({
    id: "info.update",
    path: ["Check for Updates…"],
    order: 881,
    group: "windows",
    run: async () => {
      await showAbout();
      await updater.check(true);
    },
  });
  // Options ▸ General ▸ Updates: the automatic check on or off.
  const page = plugin.settings.page("ui/updates.slint", { id: "info.updates", category: "general", title: "Updates", order: 20 });
  page.bind(() => ({ autoCheck: updater.settings.value.auto }));
  page.on("toggleAuto", (on: boolean) => updater.setAuto(on));
  updater.schedule();
  plugin.onDispose(() => {
    clearInterval(timer);
    clearTimeout(briefTimer);
  });
}
