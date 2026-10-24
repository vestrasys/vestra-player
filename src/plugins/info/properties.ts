import { basename, extname } from "node:path";
import type { AppInfo, PlaybackStats, PlayerState } from "@vestra-player/sdk";
import { API_VERSION, ffmpegVersion, vestraVersion } from "./about.ts";

/**
 * I05 Playback Properties: the rows of the three pages and their plain text. A
 * row is a group heading (`heading`, no value) or a `label: value` line; a page
 * without content is one row with an empty label ("No media open").
 *
 * i18n: needs r2-api. The rows mix fixed words (labels, units, "Not used") with
 * data and are composed here, so they stay English until the player can
 * translate such window lists; the window's own texts are @tr.
 */
export interface Row {
  label: string;
  value: string;
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

/** Rows from groups `[heading, [[label, value | null], …]]`; unknown (null/empty) values show as "-". */
function rows(groups: [string, [string, string | number | null | undefined][]][]): Row[] {
  const out: Row[] = [];
  let total = 0;
  for (const [heading, lines] of groups) {
    if (!lines.length) continue;
    out.push({ label: heading, value: "", heading: true });
    for (const [label, value] of lines) {
      const text = cut(value === null || value === undefined || value === "" ? "-" : String(value));
      total += text.length;
      if (total > PAGE_MAX) {
        out.push({ label: "", value: "(The rest is too long and was left out)", heading: false });
        return out;
      }
      out.push({ label, value: text, heading: false });
    }
  }
  return out;
}

export const message = (text: string): Row[] => [{ label: "", value: text, heading: false }];

/** `label: value` lines, a blank line before each group but the first. */
// i18n: needs r2-api (the copied / saved text)
export function pageText(page: Row[]) {
  const lines: string[] = [];
  for (const row of page) {
    if (row.heading) {
      if (lines.length) lines.push("");
      lines.push(row.label);
    } else lines.push(row.label ? `${row.label}: ${row.value}` : row.value);
  }
  return `${lines.join("\n")}\n`;
}

const isUrl = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source);
const size = (w: number | null | undefined, h: number | null | undefined) => (w && h ? `${w}×${h}` : null);
const fps = (n: number | null | undefined) => (n ? `${Number(n.toFixed(3))} fps` : null);
const kbps = (bits: number | string | null | undefined) => {
  const n = Number(bits);
  return bits !== null && bits !== undefined && bits !== "" && Number.isFinite(n) && n > 0 ? `${Math.round(n / 1000)} kbps` : null;
};
const join = (...parts: (string | null | undefined)[]) => parts.filter(Boolean).join(" · ") || null;

/** Bits and kind of an FFmpeg sample format (`floatp`, `s16`, `fltp`…). */
export function sampleFormat(format: string | null | undefined) {
  if (!format) return null;
  const bits: Record<string, string> = { u8: "8-bit", s16: "16-bit", s32: "32-bit", s64: "64-bit", flt: "32-bit float", float: "32-bit float", dbl: "64-bit float", double: "64-bit float" };
  const name = bits[format.replace(/p$/, "")];
  return name ? `${name} (${format})` : format;
}

/** `h:mm:ss`, or `mm:ss` below one hour. */
export function clock(seconds: number | null | undefined) {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return null;
  const total = Math.max(0, Math.floor(seconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

export function bytes(n: number | null | undefined) {
  if (n === null || n === undefined || !Number.isFinite(n)) return null;
  const units = ["bytes", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const exact = `${n.toLocaleString("en-US")} bytes`;
  return unit ? `${value.toFixed(2)} ${units[unit]} (${exact})` : exact;
}

/** mpv `hwdec-current` as reported: the decoder in use (`videotoolbox`, `d3d11va`…), `"no"` = software. */
export function hwdec(value: string | null | undefined) {
  if (!value) return null;
  return value === "no" ? "Not used" : value;
}

/** Playback page: what the player reports for the session now (`player.stats()`, refreshed every second). */
export function playbackPage(state: PlayerState, stats: PlaybackStats | null): Row[] {
  if (!state.source) return message("No media open");
  if (!stats) return message("Reading…");
  const { video, audio } = stats;
  return rows([
    ["Video", video ? [
      ["Decoder", video.decoder],
      ["Hardware decoding", hwdec(video.hwdec)],
      ["Input format", join(video.codec, video.pixelFormat, video.colorspace)],
      ["Size", size(video.width, video.height) && `${size(video.width, video.height)} → ${size(video.displayWidth, video.displayHeight) ?? "-"}`],
      ["Source frame rate", fps(video.fps)],
      ["Display frame rate", fps(video.displayFps)],
      ["Video bitrate", kbps(video.bitrate)],
    ] : []],
    ["Audio", audio ? [
      ["Decoder", audio.decoder],
      ["Codec", audio.codec],
      ["Sample rate", audio.sampleRate ? `${audio.sampleRate} Hz` : null],
      ["Sample format", sampleFormat(audio.format)],
      ["Channels", audio.channels ? `${audio.channels}${audio.channelLayout ? ` (${audio.channelLayout})` : ""}` : audio.channelLayout],
      ["Audio bitrate", kbps(audio.bitrate)],
    ] : []],
    ["Other", [
      ["Backend", stats.backend],
      ["Session", stats.session],
      ["Frames shown", stats.framesShown],
      ["Dropped frames", stats.framesDropped],
      ["A/V sync offset", stats.avSyncMs === null || stats.avSyncMs === undefined ? null : `${stats.avSyncMs.toFixed(1)} ms`],
      ["Cache", stats.cacheSeconds === null || stats.cacheSeconds === undefined ? null : `${stats.cacheSeconds.toFixed(1)} s`],
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

const TYPES: Record<string, string> = { video: "Video", audio: "Audio", subtitle: "Subtitles", data: "Data", attachment: "Attachment" };
const TAGS: Record<string, string> = {
  title: "Title", artist: "Artist", album_artist: "Album artist", album: "Album", date: "Date", genre: "Genre",
  track: "Track", comment: "Comment", composer: "Composer", copyright: "Copyright", encoder: "Encoder", creation_time: "Creation time",
};
/** `30000/1001` → 29.97 fps. */
const rate = (text: string | undefined) => {
  const [a, b] = (text ?? "").split("/").map(Number);
  return a && b ? fps(a / b) : null;
};
const tag = (tags: Record<string, string> | undefined, name: string) =>
  tags && Object.entries(tags).find(([key]) => key.toLowerCase() === name)?.[1];

/** File page of a local file: source, container, every stream, chapters, tags, embedded resources; dates in `locale`. */
export function filePage(path: string, file: { size: number; mtime: Date } | null, probe: Probe, locale = "en"): Row[] {
  const format = probe.format ?? {};
  const streams = probe.streams ?? [];
  const embedded = streams.filter((s) => s.codec_type === "attachment" || s.disposition?.attached_pic);
  const flags = (s: ProbeStream) => [s.disposition?.default ? "Default" : null, s.disposition?.forced ? "Forced" : null].filter(Boolean).join(", ") || "None";
  const streamGroup = (s: ProbeStream): [string, [string, string | number | null | undefined][]] => [
    `Stream #${s.index ?? "?"}: ${TYPES[s.codec_type ?? ""] ?? s.codec_type ?? "Unknown"}`,
    [
      ["Codec", join(s.codec_long_name ?? s.codec_name, s.profile)],
      ["Language", tag(s.tags, "language")],
      ["Title", tag(s.tags, "title")],
      ["Flags", flags(s)],
      ...(s.codec_type === "video" ? ([
        ["Size", size(s.width, s.height)],
        ["Frame rate", rate(s.avg_frame_rate) ?? rate(s.r_frame_rate)],
        ["Pixel format", s.pix_fmt],
        ["Color space", s.color_space],
        ["Bitrate", kbps(s.bit_rate)],
      ] as [string, string | null | undefined][]) : []),
      ...(s.codec_type === "audio" ? ([
        ["Sample rate", s.sample_rate ? `${s.sample_rate} Hz` : null],
        ["Channels", s.channels ? `${s.channels}${s.channel_layout ? ` (${s.channel_layout})` : ""}` : null],
        ["Sample format", sampleFormat(s.sample_fmt)],
        ["Bitrate", kbps(s.bit_rate)],
      ] as [string, string | null | undefined][]) : []),
    ],
  ];
  const tags = Object.entries(format.tags ?? {});
  return rows([
    ["Source", [
      ["Path", path],
      ["File size", bytes(file?.size)],
      ["Modified", file ? file.mtime.toLocaleString(locale, { hour12: false }) : null],
    ]],
    ["Container", [
      ["Format", format.format_long_name ? `${format.format_long_name} (${format.format_name})` : format.format_name],
      ["Duration", clock(Number(format.duration))],
      ["Overall bitrate", kbps(format.bit_rate)],
      ["Streams", streams.length],
      ["Chapters", (probe.chapters ?? []).length],
    ]],
    ...streams.filter((s) => !embedded.includes(s)).map(streamGroup),
    ["Metadata", tags.map(([key, value]) => [TAGS[key.toLowerCase()] ?? key, value] as [string, string])],
    ["Embedded resources", embedded.map((s) =>
      s.codec_type === "attachment"
        ? [`Attachment #${s.index}`, join(tag(s.tags, "filename") ?? "(no file name)", tag(s.tags, "mimetype"))] as [string, string | null]
        : [`Cover #${s.index}`, join(s.codec_name, size(s.width, s.height))] as [string, string | null])],
  ]);
}

/** File page of a network source: only what the player reports. */
export function remotePage(state: PlayerState, stats: PlaybackStats | null): Row[] {
  const { video, audio } = stats ?? { video: null, audio: null };
  return [
    ...message("Network source: only what the player reports"),
    ...rows([
      ["Source", [
        ["Address", state.source],
        ["Duration", state.duration === null ? "Live" : clock(state.duration)],
      ]],
      ["Streams", [
        ["Video", video ? join(video.codec, size(video.width, video.height), fps(video.fps)) : null],
        ["Audio", audio ? join(audio.codec, audio.sampleRate ? `${audio.sampleRate} Hz` : null, audio.channels ? `${audio.channels} channels` : null) : null],
        ["Audio tracks", state.audioTracks?.length ?? null],
      ]],
    ]),
  ];
}

/** What `sysinfo.ts` read; every field may be unknown. */
export interface SystemFacts {
  /** `macOS 26.2 (25C56)`, `Windows 11 23H2 (22631.4460)`. */
  os: string | null;
  model: string | null;
  cpu: string | null;
  memory: number | null;
  freeMemory: number | null;
  displays: string[];
}

/** System page. Vestra Player, mpv and FFmpeg versions come from the core (`plugin.app`); one it does not know shows Unknown. */
export function systemPage(app: AppInfo, facts: SystemFacts | null): Row[] {
  const gb = (n: number | null) => (n === null ? null : `${(n / 1024 ** 3).toFixed(1)} GB`);
  return rows([
    ["Player", [
      ["Vestra Player version", vestraVersion(app)],
      ["Plugin API version", API_VERSION],
      ["mpv version", app.components.mpv || "Unknown"],
      ["FFmpeg version", ffmpegVersion(app) || "Unknown"],
    ]],
    ["System", facts ? [
      ["System version", facts.os],
      ["Model", facts.model],
      ["CPU", facts.cpu],
      ["Total memory", gb(facts.memory)],
      ["Free memory", gb(facts.freeMemory)],
      ...(facts.displays.length ? facts.displays.map((d, i) => [facts.displays.length > 1 ? `Display ${i + 1}` : "Display", d] as [string, string]) : [["Display", null] as [string, null]]),
    ] : [["Status", "Reading…"]]],
  ]);
}

/** `movie info.txt`, or `Vestra Player info.txt` without media. */
// i18n: needs r2-api (saveFile fileName: a translated word around data)
export function saveName(source: string | null) {
  if (!source) return "Vestra Player info.txt";
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
  return `${stem} info.txt`;
}

export { isUrl };
