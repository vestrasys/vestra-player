import { basename, extname } from "node:path";
import type { AppInfo, PlaybackStats, PlayerState } from "../../sdk/plugin/index.ts";
import { API_VERSION, ffmpegVersion, vestraVersion } from "./about.ts";

/**
 * I05 播放属性: the rows of the three pages and their plain text. A row is a
 * group heading (`heading`, no value) or a `label：value` line; a page without
 * content is one row with an empty label (「未打开媒体」).
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
        out.push({ label: "", value: "（其余内容过长，已省略）", heading: false });
        return out;
      }
      out.push({ label, value: text, heading: false });
    }
  }
  return out;
}

export const message = (text: string): Row[] => [{ label: "", value: text, heading: false }];

/** `键：值` lines, a blank line before each group but the first. */
export function pageText(page: Row[]) {
  const lines: string[] = [];
  for (const row of page) {
    if (row.heading) {
      if (lines.length) lines.push("");
      lines.push(row.label);
    } else lines.push(row.label ? `${row.label}：${row.value}` : row.value);
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
  const bits: Record<string, string> = { u8: "8 位", s16: "16 位", s32: "32 位", s64: "64 位", flt: "32 位浮点", float: "32 位浮点", dbl: "64 位浮点", double: "64 位浮点" };
  const name = bits[format.replace(/p$/, "")];
  return name ? `${name}（${format}）` : format;
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
  const units = ["字节", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const exact = `${n.toLocaleString("en-US")} 字节`;
  return unit ? `${value.toFixed(2)} ${units[unit]}（${exact}）` : exact;
}

/** mpv `hwdec-current` as reported: the decoder in use (`videotoolbox`, `d3d11va`…), `"no"` = software. */
export function hwdec(value: string | null | undefined) {
  if (!value) return null;
  return value === "no" ? "未使用" : value;
}

/** 播放信息: what the player reports for the session now (`player.stats()`, refreshed every second). */
export function playbackPage(state: PlayerState, stats: PlaybackStats | null): Row[] {
  if (!state.source) return message("未打开媒体");
  if (!stats) return message("正在读取…");
  const { video, audio } = stats;
  return rows([
    ["视频", video ? [
      ["解码器", video.decoder],
      ["硬件解码", hwdec(video.hwdec)],
      ["输入格式", join(video.codec, video.pixelFormat, video.colorspace)],
      ["尺寸", size(video.width, video.height) && `${size(video.width, video.height)} → ${size(video.displayWidth, video.displayHeight) ?? "-"}`],
      ["源帧率", fps(video.fps)],
      ["当前帧率", fps(video.displayFps)],
      ["视频码率", kbps(video.bitrate)],
    ] : []],
    ["音频", audio ? [
      ["解码器", audio.decoder],
      ["编码", audio.codec],
      ["采样率", audio.sampleRate ? `${audio.sampleRate} Hz` : null],
      ["采样位数", sampleFormat(audio.format)],
      ["声道数", audio.channels ? `${audio.channels}${audio.channelLayout ? `（${audio.channelLayout}）` : ""}` : audio.channelLayout],
      ["音频码率", kbps(audio.bitrate)],
    ] : []],
    ["其它", [
      ["后端", stats.backend],
      ["会话", stats.session],
      ["已显示帧", stats.framesShown],
      ["丢帧", stats.framesDropped],
      ["音画同步偏差", stats.avSyncMs === null || stats.avSyncMs === undefined ? null : `${stats.avSyncMs.toFixed(1)} ms`],
      ["缓冲时长", stats.cacheSeconds === null || stats.cacheSeconds === undefined ? null : `${stats.cacheSeconds.toFixed(1)} 秒`],
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

const TYPES: Record<string, string> = { video: "视频", audio: "音频", subtitle: "字幕", data: "数据", attachment: "附件" };
const TAGS: Record<string, string> = {
  title: "标题", artist: "艺术家", album_artist: "专辑艺术家", album: "专辑", date: "日期", genre: "流派",
  track: "音轨号", comment: "注释", composer: "作曲", copyright: "版权", encoder: "编码器", creation_time: "创建时间",
};
/** `30000/1001` → 29.97 fps. */
const rate = (text: string | undefined) => {
  const [a, b] = (text ?? "").split("/").map(Number);
  return a && b ? fps(a / b) : null;
};
const tag = (tags: Record<string, string> | undefined, name: string) =>
  tags && Object.entries(tags).find(([key]) => key.toLowerCase() === name)?.[1];

/** 文件信息 of a local file: source, container, every stream, chapters, tags, embedded resources. */
export function filePage(path: string, file: { size: number; mtime: Date } | null, probe: Probe): Row[] {
  const format = probe.format ?? {};
  const streams = probe.streams ?? [];
  const embedded = streams.filter((s) => s.codec_type === "attachment" || s.disposition?.attached_pic);
  const flags = (s: ProbeStream) => [s.disposition?.default ? "默认" : null, s.disposition?.forced ? "强制" : null].filter(Boolean).join("、") || "无";
  const streamGroup = (s: ProbeStream): [string, [string, string | number | null | undefined][]] => [
    `流 #${s.index ?? "?"}：${TYPES[s.codec_type ?? ""] ?? s.codec_type ?? "未知"}`,
    [
      ["编码", join(s.codec_long_name ?? s.codec_name, s.profile)],
      ["语言", tag(s.tags, "language")],
      ["标题", tag(s.tags, "title")],
      ["标记", flags(s)],
      ...(s.codec_type === "video" ? ([
        ["尺寸", size(s.width, s.height)],
        ["帧率", rate(s.avg_frame_rate) ?? rate(s.r_frame_rate)],
        ["像素格式", s.pix_fmt],
        ["色彩空间", s.color_space],
        ["码率", kbps(s.bit_rate)],
      ] as [string, string | null | undefined][]) : []),
      ...(s.codec_type === "audio" ? ([
        ["采样率", s.sample_rate ? `${s.sample_rate} Hz` : null],
        ["声道", s.channels ? `${s.channels}${s.channel_layout ? `（${s.channel_layout}）` : ""}` : null],
        ["采样格式", sampleFormat(s.sample_fmt)],
        ["码率", kbps(s.bit_rate)],
      ] as [string, string | null | undefined][]) : []),
    ],
  ];
  const tags = Object.entries(format.tags ?? {});
  return rows([
    ["来源", [
      ["路径", path],
      ["文件大小", bytes(file?.size)],
      ["修改时间", file ? file.mtime.toLocaleString("zh-CN", { hour12: false }) : null],
    ]],
    ["容器", [
      ["格式", format.format_long_name ? `${format.format_long_name}（${format.format_name}）` : format.format_name],
      ["总时长", clock(Number(format.duration))],
      ["总码率", kbps(format.bit_rate)],
      ["流数量", streams.length],
      ["章节数", (probe.chapters ?? []).length],
    ]],
    ...streams.filter((s) => !embedded.includes(s)).map(streamGroup),
    ["元数据", tags.map(([key, value]) => [TAGS[key.toLowerCase()] ?? key, value] as [string, string])],
    ["内嵌资源", embedded.map((s) =>
      s.codec_type === "attachment"
        ? [`附件 #${s.index}`, join(tag(s.tags, "filename") ?? "（无文件名）", tag(s.tags, "mimetype"))] as [string, string | null]
        : [`封面 #${s.index}`, join(s.codec_name, size(s.width, s.height))] as [string, string | null])],
  ]);
}

/** 文件信息 of a network source: only what the player reports. */
export function remotePage(state: PlayerState, stats: PlaybackStats | null): Row[] {
  const { video, audio } = stats ?? { video: null, audio: null };
  return [
    ...message("网络来源：只显示播放器报告的信息"),
    ...rows([
      ["来源", [
        ["地址", state.source],
        ["总时长", state.duration === null ? "直播" : clock(state.duration)],
      ]],
      ["流", [
        ["视频", video ? join(video.codec, size(video.width, video.height), fps(video.fps)) : null],
        ["音频", audio ? join(audio.codec, audio.sampleRate ? `${audio.sampleRate} Hz` : null, audio.channels ? `${audio.channels} 声道` : null) : null],
        ["音轨数", state.audioTracks?.length ?? null],
      ]],
    ]),
  ];
}

/** What `sysinfo.ts` read; every field may be unknown. */
export interface SystemFacts {
  /** `macOS 26.2（25C56）`, `Windows 11 23H2（22631.4460）`. */
  os: string | null;
  model: string | null;
  cpu: string | null;
  memory: number | null;
  freeMemory: number | null;
  displays: string[];
}

/** 系统信息. Vestra Player, mpv and FFmpeg versions come from the core (`plugin.app`); one it does not know shows 未知. */
export function systemPage(app: AppInfo, facts: SystemFacts | null): Row[] {
  const gb = (n: number | null) => (n === null ? null : `${(n / 1024 ** 3).toFixed(1)} GB`);
  return rows([
    ["播放器", [
      ["Vestra Player 版本", vestraVersion(app)],
      ["插件 API 版本", API_VERSION],
      ["mpv 版本", app.components.mpv || "未知"],
      ["FFmpeg 版本", ffmpegVersion(app)],
    ]],
    ["系统", facts ? [
      ["系统版本", facts.os],
      ["机型", facts.model],
      ["CPU", facts.cpu],
      ["内存总量", gb(facts.memory)],
      ["可用内存", gb(facts.freeMemory)],
      ...(facts.displays.length ? facts.displays.map((d, i) => [facts.displays.length > 1 ? `显示器 ${i + 1}` : "显示器", d] as [string, string]) : [["显示器", null] as [string, null]]),
    ] : [["状态", "正在读取…"]]],
  ]);
}

/** `movie 信息.txt`, or `Vestra Player 信息.txt` without media. */
export function saveName(source: string | null) {
  if (!source) return "Vestra Player 信息.txt";
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
  return `${stem} 信息.txt`;
}

export { isUrl };
