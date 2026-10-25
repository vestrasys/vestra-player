import { basename, extname } from "node:path";
import type { AppInfo, I18n, PlaybackStats, PlayerState, TextArg, TextValue } from "@vestra-player/sdk";
import { API_VERSION, ffmpegVersion, vestraVersion } from "./about.ts";

/**
 * I05 Playback Properties: the rows of the three pages and their plain text. A
 * row is a group heading (`heading`, no value) or a `label: value` line; a page
 * without content is one row with an empty label ("No media open"). Labels and
 * values are text values (`plugin.i18n.text`) or data; the player translates
 * them in the window, `pageText` resolves them for the clipboard and files.
 */
export interface Row {
  label: TextArg;
  value: TextArg;
  heading: boolean;
}

/** Each value is at most this many characters, cut with "…". */
export const VALUE_MAX = 512;
/** All values of one page together stay under the 64 KB string limit. */
export const PAGE_MAX = 60_000;

export const cut = (text: string, max = VALUE_MAX) => {
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
};

type Value = TextArg | null | undefined;
type Group = [TextArg, [TextArg, Value][]];

/** Rows from groups `[heading, [[label, value | null], …]]`; unknown (null/empty) values show as "-". */
function rows(i18n: I18n, groups: Group[]): Row[] {
  const out: Row[] = [];
  let total = 0;
  for (const [heading, lines] of groups) {
    if (!lines.length) continue;
    out.push({ label: heading, value: "", heading: true });
    for (const [label, value] of lines) {
      const shown = value === null || value === undefined || value === "" ? "-" : typeof value === "object" ? value : cut(String(value));
      total += typeof shown === "string" ? shown.length : JSON.stringify(shown).length;
      if (total > PAGE_MAX) {
        out.push({ label: "", value: i18n.text("(The rest is too long and was left out)"), heading: false });
        return out;
      }
      out.push({ label, value: shown, heading: false });
    }
  }
  return out;
}

export const message = (value: TextArg): Row[] => [{ label: "", value, heading: false }];

/** `label: value` lines, a blank line before each group but the first: the page as it leaves the player. */
export function pageText(i18n: I18n, page: Row[]) {
  const { text, list, resolve } = i18n;
  const lines: TextArg[] = [];
  for (const row of page) {
    if (row.heading) {
      if (lines.length) lines.push("");
      lines.push(row.label);
    } else lines.push(row.label === "" ? row.value : text("{}: {}", [row.label, row.value]));
  }
  return `${resolve(list(lines, "lines"))}\n`;
}

const isUrl = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source);
const size = (w: number | null | undefined, h: number | null | undefined) => (w && h ? `${w}×${h}` : null);
const fps = (n: number | null | undefined) => (n ? `${Number(n.toFixed(3))} fps` : null);
const kbps = (bits: number | string | null | undefined) => {
  const n = Number(bits);
  return bits !== null && bits !== undefined && bits !== "" && Number.isFinite(n) && n > 0 ? `${Math.round(n / 1000)} kbps` : null;
};
/** The known parts joined with " · ". */
function join(i18n: I18n, ...parts: Value[]): TextArg | null {
  const known = parts.filter((part): part is TextArg => part !== null && part !== undefined && part !== "");
  if (!known.length) return null;
  return known.every((part) => typeof part !== "object")
    ? known.join(" · ")
    : known.reduce((line, part) => i18n.text("{} · {}", [line, part]));
}
const has = <T extends object>(table: T, key: string): key is Extract<keyof T, string> => Object.hasOwn(table, key);

/** Bits and kind of an FFmpeg sample format (`floatp`, `s16`, `fltp`…), the format in parentheses. */
const BITS = {
  u8: "8-bit ({})", s16: "16-bit ({})", s32: "32-bit ({})", s64: "64-bit ({})",
  flt: "32-bit float ({})", float: "32-bit float ({})", dbl: "64-bit float ({})", double: "64-bit float ({})",
} as const;
export function sampleFormat(i18n: I18n, format: string | null | undefined): TextArg | null {
  if (!format) return null;
  const kind = format.replace(/p$/, "");
  return has(BITS, kind) ? i18n.text(BITS[kind], [format]) : format;
}

/** `h:mm:ss`, or `mm:ss` below one hour. */
export function clock(seconds: number | null | undefined) {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return null;
  const total = Math.max(0, Math.floor(seconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

/** `2.00 KB (2,048 bytes)`: the larger unit and the exact count. */
const UNITS = ["{} KB ({})", "{} MB ({})", "{} GB ({})", "{} TB ({})"] as const;
export function bytes(i18n: I18n, n: number | null | undefined): TextArg | null {
  if (n === null || n === undefined || !Number.isFinite(n)) return null;
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length) {
    value /= 1024;
    unit++;
  }
  const exact = i18n.text("{} bytes", [n.toLocaleString("en-US")]);
  return unit ? i18n.text(UNITS[unit - 1]!, [value.toFixed(2), exact]) : exact;
}

/** mpv `hwdec-current` as reported: the decoder in use (`videotoolbox`, `d3d11va`…), `"no"` = software. */
export function hwdec(i18n: I18n, value: string | null | undefined): TextArg | null {
  if (!value) return null;
  return value === "no" ? i18n.text("Not used") : value;
}

/** Playback page: what the player reports for the session now (`player.stats()`, refreshed every second). */
export function playbackPage(i18n: I18n, state: PlayerState, stats: PlaybackStats | null): Row[] {
  const { text } = i18n;
  if (!state.source) return message(text("No media open"));
  if (!stats) return message(text("Reading…"));
  const { video, audio } = stats;
  return rows(i18n, [
    [text("Video"), video ? [
      [text("Decoder"), video.decoder],
      [text("Hardware decoding"), hwdec(i18n, video.hwdec)],
      [text("Input format"), join(i18n, video.codec, video.pixelFormat, video.colorspace)],
      [text("Size"), size(video.width, video.height) && `${size(video.width, video.height)} → ${size(video.displayWidth, video.displayHeight) ?? "-"}`],
      [text("Source frame rate"), fps(video.fps)],
      [text("Display frame rate"), fps(video.displayFps)],
      [text("Video bitrate"), kbps(video.bitrate)],
    ] : []],
    [text("Audio"), audio ? [
      [text("Decoder"), audio.decoder],
      [text("Codec"), audio.codec],
      [text("Sample rate"), audio.sampleRate ? `${audio.sampleRate} Hz` : null],
      [text("Bit depth"), sampleFormat(i18n, audio.format)],
      [text("Channel count"), audio.channels ? (audio.channelLayout ? text("{} ({})", [audio.channels, audio.channelLayout]) : audio.channels) : audio.channelLayout],
      [text("Audio bitrate"), kbps(audio.bitrate)],
    ] : []],
    [text("Other"), [
      [text("Backend"), stats.backend],
      [text("Session"), stats.session],
      [text("Frames shown"), stats.framesShown],
      [text("Dropped frames"), stats.framesDropped],
      [text("A/V sync offset"), stats.avSyncMs === null || stats.avSyncMs === undefined ? null : `${stats.avSyncMs.toFixed(1)} ms`],
      [text("Cache"), stats.cacheSeconds === null || stats.cacheSeconds === undefined ? null : text("{} s", [stats.cacheSeconds.toFixed(1)])],
    ]],
  ]);
}

/** The parts of `ffprobe -show_format -show_streams -show_chapters -of json` used here. */
export interface Probe {
  format?: { format_name?: string; format_long_name?: string; duration?: string; bit_rate?: string; tags?: Record<string, string> };
  streams?: ProbeStream[];
  chapters?: unknown[];
}
interface ProbeStream {
  index?: number; codec_type?: string; codec_name?: string; codec_long_name?: string; profile?: string;
  width?: number; height?: number; pix_fmt?: string; color_space?: string; avg_frame_rate?: string; r_frame_rate?: string;
  sample_rate?: string; channels?: number; channel_layout?: string; sample_fmt?: string; bit_rate?: string;
  disposition?: Record<string, number>; tags?: Record<string, string>;
}

const TYPES = { video: "Video", audio: "Audio", subtitle: "Subtitles", data: "Data", attachment: "Attachment" } as const;
const TAGS = {
  title: "Title", artist: "Artist", album_artist: "Album artist", album: "Album", date: "Date", genre: "Genre",
  track: "Track", comment: "Comment", composer: "Composer", copyright: "Copyright", encoder: "Encoder", creation_time: "Creation time",
} as const;
/** `30000/1001` → 29.97 fps. */
const rate = (text: string | undefined) => {
  const [a, b] = (text ?? "").split("/").map(Number);
  return a && b ? fps(a / b) : null;
};
const tag = (tags: Record<string, string> | undefined, name: string) =>
  tags && Object.entries(tags).find(([key]) => key.toLowerCase() === name)?.[1];
/** `value (detail)`, or the one that is known. */
const detail = (i18n: I18n, value: Value, more: Value) => (value && more ? i18n.text("{} ({})", [value, more]) : value || more);

/** File page of a local file: source, container, every stream, chapters, tags, embedded resources; dates in `locale`. */
export function filePage(i18n: I18n, path: string, file: { size: number; mtime: Date } | null, probe: Probe, locale = "en"): Row[] {
  const { text, list } = i18n;
  const format = probe.format ?? {};
  const streams = probe.streams ?? [];
  const embedded = streams.filter((s) => s.codec_type === "attachment" || s.disposition?.attached_pic);
  const flags = (s: ProbeStream) => {
    const set = [s.disposition?.default ? text("Default") : null, s.disposition?.forced ? text("Forced") : null].filter((flag) => flag !== null);
    return set.length ? list(set) : text("None");
  };
  const type = (s: ProbeStream) => (has(TYPES, s.codec_type ?? "") ? text(TYPES[s.codec_type as keyof typeof TYPES]) : s.codec_type || text("Unknown"));
  const streamGroup = (s: ProbeStream): Group => [
    text("Stream #{}: {}", [s.index ?? "?", type(s)]),
    [
      [text("Codec"), join(i18n, s.codec_long_name ?? s.codec_name, s.profile)],
      [text("Language"), tag(s.tags, "language")],
      [text("Title"), tag(s.tags, "title")],
      [text("Flags"), flags(s)],
      ...(s.codec_type === "video" ? ([
        [text("Size"), size(s.width, s.height)],
        [text("Frame rate"), rate(s.avg_frame_rate) ?? rate(s.r_frame_rate)],
        [text("Pixel format"), s.pix_fmt],
        [text("Color space"), s.color_space],
        [text("Bitrate"), kbps(s.bit_rate)],
      ] as [TextArg, Value][]) : []),
      ...(s.codec_type === "audio" ? ([
        [text("Sample rate"), s.sample_rate ? `${s.sample_rate} Hz` : null],
        [text("Channels"), s.channels ? detail(i18n, s.channels, s.channel_layout) : null],
        [text("Sample format"), sampleFormat(i18n, s.sample_fmt)],
        [text("Bitrate"), kbps(s.bit_rate)],
      ] as [TextArg, Value][]) : []),
    ],
  ];
  const tags = Object.entries(format.tags ?? {});
  const tagName = (key: string): TextArg => (has(TAGS, key.toLowerCase()) ? text(TAGS[key.toLowerCase() as keyof typeof TAGS]) : key);
  return rows(i18n, [
    [text("Source"), [
      [text("Path"), path],
      [text("File size"), bytes(i18n, file?.size)],
      [text("Modified"), file ? file.mtime.toLocaleString(locale, { hour12: false }) : null],
    ]],
    [text("Container"), [
      [text("Format"), format.format_long_name ? detail(i18n, format.format_long_name, format.format_name) : format.format_name],
      [text("Duration"), clock(Number(format.duration))],
      [text("Overall bitrate"), kbps(format.bit_rate)],
      [text("Stream count"), streams.length],
      [text("Chapters"), (probe.chapters ?? []).length],
    ]],
    ...streams.filter((s) => !embedded.includes(s)).map(streamGroup),
    [text("Metadata"), tags.map(([key, value]): [TextArg, Value] => [tagName(key), value])],
    [text("Embedded resources"), embedded.map((s): [TextArg, Value] =>
      s.codec_type === "attachment"
        ? [text("Attachment #{}", [s.index ?? "?"]), join(i18n, tag(s.tags, "filename") ?? text("(no file name)"), tag(s.tags, "mimetype"))]
        : [text("Cover #{}", [s.index ?? "?"]), join(i18n, s.codec_name, size(s.width, s.height))])],
  ]);
}

/** File page of a network source: only what the player reports. */
export function remotePage(i18n: I18n, state: PlayerState, stats: PlaybackStats | null): Row[] {
  const { text } = i18n;
  const { video, audio } = stats ?? { video: null, audio: null };
  return [
    ...message(text("Network source: only what the player reports")),
    ...rows(i18n, [
      [text("Source"), [
        [text("Address"), state.source],
        [text("Duration"), state.duration === null ? text("Live") : clock(state.duration)],
      ]],
      [text("Streams"), [
        [text("Video"), video ? join(i18n, video.codec, size(video.width, video.height), fps(video.fps)) : null],
        [text("Audio"), audio ? join(i18n, audio.codec, audio.sampleRate ? `${audio.sampleRate} Hz` : null, audio.channels ? text("{} channels", [audio.channels]) : null) : null],
        [text("Audio tracks"), state.audioTracks?.length ?? null],
      ]],
    ]),
  ];
}

/** What `sysinfo.ts` read; every field may be unknown. */
export interface SystemFacts {
  /** `macOS 26.2 (25C56)`, `Windows 11 23H2 (22631.4460)`. */
  os: TextArg | null;
  model: string | null;
  /** The CPU's name, and how many cores it has (0 = not known). */
  cpu: string | null;
  cores: number;
  memory: number | null;
  freeMemory: number | null;
  displays: string[];
}

/** System page. Vestra Player, mpv and FFmpeg versions come from the core (`plugin.app`); one it does not know shows Unknown. */
export function systemPage(i18n: I18n, app: AppInfo, facts: SystemFacts | null): Row[] {
  const { text } = i18n;
  const gb = (n: number | null) => (n === null ? null : `${(n / 1024 ** 3).toFixed(1)} GB`);
  const cpu = facts?.cpu && facts.cores ? text("{} ({} cores)", [facts.cpu, facts.cores]) : facts?.cpu;
  return rows(i18n, [
    [text("Player"), [
      [text("Vestra Player version"), vestraVersion(i18n, app)],
      [text("Plugin API version"), API_VERSION],
      [text("mpv version"), app.components.mpv || text("Unknown")],
      [text("FFmpeg version"), ffmpegVersion(app) || text("Unknown")],
    ]],
    [text("System"), facts ? [
      [text("System version"), facts.os],
      [text("Model"), facts.model],
      [text("CPU"), cpu],
      [text("Total memory"), gb(facts.memory)],
      [text("Free memory"), gb(facts.freeMemory)],
      ...(facts.displays.length
        ? facts.displays.map((d, i): [TextArg, Value] => [facts.displays.length > 1 ? text("Display {}", [i + 1]) : text("Display"), d])
        : [[text("Display"), null] as [TextArg, Value]]),
    ] : [[text("Status"), text("Reading…")]]],
  ]);
}

/** `movie info.txt`, or `Vestra Player info.txt` without media: the proposed name of Save As…. */
export function saveName(i18n: I18n, source: string | null): TextValue {
  if (!source) return i18n.text("Vestra Player info.txt");
  let name = source;
  if (isUrl(source)) {
    try {
      name = new URL(source).pathname.split("/").filter(Boolean).at(-1) ?? new URL(source).hostname;
      name = decodeURIComponent(name);
    } catch {
      name = "Vestra Player";
    }
  } else name = basename(source);
  const stem = basename(name, extname(name)).replace(/[\\/:*?"<>|]/g, "_") || "Vestra Player";
  // TRANSLATORS: only the part before the extension; {} is the media's file name.
  return i18n.text("{} info.txt", [stem]);
}

export { isUrl };
