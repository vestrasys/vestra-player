// Subtitle files → what the player accepts (D18): UTF-8 SRT, or ASS/SSA as is.
// The same file is in subtitle-online (plugins share no code); keep the two copies equal
// (here the errors and notes are status ids, messages.ts, since the i18n migration).
import { open } from "node:fs/promises";
import { Failure } from "./messages.ts";
import type { NoteId } from "./messages.ts";

/** Largest file read from disk. */
export const MAX_FILE = 8 * 1024 * 1024;
/** The player's upload limit (reference §9). */
export const MAX_UPLOAD = 2 * 1024 * 1024;

/**
 * Default encoding choices for files that are neither BOM-marked nor valid
 * UTF-8; GB18030 first. Their names are in ui/settings.slint, in this order.
 */
export const ENCODINGS = [
  { id: "gb18030" },
  { id: "big5" },
  { id: "shift_jis" },
  { id: "euc-kr" },
  { id: "windows-1252" },
  { id: "windows-1251" },
  { id: "iso-8859-1" },
] as const;
export type Encoding = (typeof ENCODINGS)[number]["id"];
export const DEFAULT_ENCODING: Encoding = "gb18030";
export const isEncoding = (value: unknown): value is Encoding => ENCODINGS.some((e) => e.id === value);

export const NOTE_FONTS: NoteId = "noteFonts";
export const NOTE_ENCODING: NoteId = "noteEncoding";
export const NOTE_FPS: NoteId = "noteFps";

/** One track to upload. SAMI files with several languages give one per language. */
export type Converted = { format: "srt" | "ass"; bytes: Buffer; language: string | null };
export type Conversion = { kind: string; tracks: Converted[]; notes: NoteId[] };

/** Read a regular file of 1 byte – 8 MiB. Errors are `Failure`s (messages.ts). */
export async function readFileBytes(path: string): Promise<Buffer> {
  const file = await open(path, "r").catch(() => {
    throw new Failure("readFailed");
  });
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size < 1 || stat.size > MAX_FILE)
      throw new Failure("notRegular");
    const buffer = Buffer.alloc(MAX_FILE + 1);
    let length = 0;
    while (length < buffer.length) {
      const result = await file.read(buffer, length, buffer.length - length, null);
      if (!result.bytesRead) break;
      length += result.bytesRead;
    }
    if (length < 1 || length > MAX_FILE) throw new Failure("emptyOrLarge");
    return buffer.subarray(0, length);
  } finally {
    await file.close();
  }
}

/**
 * Text of a subtitle file: a BOM (UTF-8, UTF-16LE/BE) wins, then strict UTF-8,
 * then `fallback`. `utf8` is true when the bytes were already UTF-8 without a BOM.
 */
export function decode(bytes: Uint8Array, fallback: Encoding) {
  const b = bytes;
  let text: string;
  let utf8 = false;
  if (b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf) text = new TextDecoder("utf-8").decode(b.subarray(3));
  else if (b[0] === 0xff && b[1] === 0xfe) text = new TextDecoder("utf-16le").decode(b.subarray(2));
  else if (b[0] === 0xfe && b[1] === 0xff) text = new TextDecoder("utf-16be").decode(b.subarray(2));
  else {
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(b);
      utf8 = true;
    } catch {
      text = new TextDecoder(fallback).decode(b);
    }
  }
  const replaced = [...text.matchAll(/�/g)].length;
  return { text, utf8, suspicious: text.length > 0 && replaced / text.length > 0.01 };
}

const isBitmap = (b: Uint8Array) =>
  (b[0] === 0x50 && b[1] === 0x47) || // PGS .sup ("PG")
  (b[0] === 0x00 && b[1] === 0x00 && b[2] === 0x01 && b[3] === 0xba); // VobSub .sub (MPEG-PS pack)

/**
 * Detect the format by content (the file name is only a hint) and convert:
 * SRT and ASS/SSA pass through (ASS without [Fonts]/[Graphics]); WebVTT, SAMI
 * and MicroDVD become SRT. Throws a `Failure` for unsupported files.
 */
export function convert(bytes: Uint8Array, options: { fallback: Encoding; fps: number | null }): Conversion {
  if (isBitmap(bytes)) throw new Failure("bitmap");
  const decoded = decode(bytes, options.fallback);
  const text = decoded.text.replace(/^﻿/, "");
  const notes: NoteId[] = decoded.suspicious ? [NOTE_ENCODING] : [];
  if (/^\s*# VobSub index file/i.test(text)) throw new Failure("bitmap");
  if (!decoded.utf8 && /\u0000/.test(text)) throw new Failure("bitmap");
  const keep = () => (decoded.utf8 ? Buffer.from(bytes) : Buffer.from(text, "utf8"));
  let kind: string;
  let tracks: Converted[];
  if (/^\s*\[Script Info\]/i.test(text) && /^\[Events\]/im.test(text)) {
    kind = /ScriptType:\s*v4\.00\+/i.test(text) ? "ASS" : "SSA";
    const stripped = stripAttachments(text);
    if (stripped !== text) notes.push(NOTE_FONTS);
    tracks = [{ format: "ass", bytes: stripped === text ? keep() : Buffer.from(stripped, "utf8"), language: null }];
  } else if (/^WEBVTT/.test(text)) {
    kind = "WebVTT";
    tracks = [{ format: "srt", bytes: Buffer.from(toSrt(parseVtt(text)), "utf8"), language: null }];
  } else if (/<SAMI\b/i.test(text)) {
    kind = "SAMI";
    tracks = parseSami(text).map(({ language, cues }) => ({ format: "srt", bytes: Buffer.from(toSrt(cues), "utf8"), language }));
  } else if (/^\s*\{\d+\}\{\d*\}/.test(text)) {
    kind = "MicroDVD";
    const { cues, guessed } = parseMicroDvd(text, options.fps);
    if (guessed) notes.push(NOTE_FPS);
    tracks = [{ format: "srt", bytes: Buffer.from(toSrt(cues), "utf8"), language: null }];
  } else if (SRT_TIME.test(text)) {
    kind = "SRT";
    tracks = [{ format: "srt", bytes: keep(), language: null }];
  } else throw new Failure("unknownFormat");
  tracks = tracks.filter((t) => t.bytes.length > 0);
  if (!tracks.length) throw new Failure("noCues");
  if (tracks.some((t) => t.bytes.length > MAX_UPLOAD)) throw new Failure("tooLarge");
  return { kind, tracks, notes };
}

// ------------------------------------------------------------------ cues
export type Cue = { start: number; end: number; text: string };

const SRT_TIME = /(\d+):(\d{1,2}):(\d{1,2})[,.](\d{1,3})\s*-->\s*(\d+):(\d{1,2}):(\d{1,2})[,.](\d{1,3})/;
const seconds = (h: string, m: string, s: string, f: string) =>
  Number(h) * 3600 + Number(m) * 60 + Number(s) + Number(f.padEnd(3, "0")) / 1000;

function stamp(t: number) {
  const ms = Math.max(0, Math.round(t * 1000));
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(Math.floor(ms / 3600000))}:${pad(Math.floor(ms / 60000) % 60)}:${pad(Math.floor(ms / 1000) % 60)},${pad(ms % 1000, 3)}`;
}
export const toSrt = (cues: Cue[]) =>
  cues
    .filter((c) => c.text.trim() && c.end > c.start)
    .map((c, i) => `${i + 1}\n${stamp(c.start)} --> ${stamp(c.end)}\n${c.text.trim()}\n`)
    .join("\n");

/** SRT cues in file order; malformed blocks are skipped. */
export function parseSrt(text: string): Cue[] {
  const cues: Cue[] = [];
  for (const block of text.replace(/\r\n?/g, "\n").split(/\n\s*\n/)) {
    const lines = block.split("\n");
    const at = lines.findIndex((line) => SRT_TIME.test(line));
    if (at < 0) continue;
    const m = SRT_TIME.exec(lines[at]!)!;
    cues.push({
      start: seconds(m[1]!, m[2]!, m[3]!, m[4]!),
      end: seconds(m[5]!, m[6]!, m[7]!, m[8]!),
      text: lines.slice(at + 1).join("\n").replace(/<[^>]+>/g, "").trim(),
    });
  }
  return cues;
}

/** ASS/SSA `Dialogue:` lines in the [Events] section, per its Format line; tags removed. */
export function parseAss(text: string): Cue[] {
  const cues: Cue[] = [];
  let events = false;
  let format = ["layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"];
  for (const raw of text.replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw.trim();
    if (/^\[.*\]$/.test(line)) {
      events = /^\[Events\]$/i.test(line);
      continue;
    }
    if (!events) continue;
    const field = /^(\w+):\s*(.*)$/.exec(line);
    if (!field) continue;
    if (/^format$/i.test(field[1]!)) format = field[2]!.split(",").map((f) => f.trim().toLowerCase());
    if (!/^dialogue$/i.test(field[1]!)) continue;
    const parts = field[2]!.split(",");
    const value = (name: string) => parts[format.indexOf(name)]?.trim() ?? "";
    const body = parts.slice(format.length - 1).join(",");
    const time = (v: string) => {
      const m = /^(\d+):(\d{1,2}):(\d{1,2})[.:](\d{1,3})$/.exec(v);
      return m ? seconds(m[1]!, m[2]!, m[3]!, m[4]!.padEnd(2, "0").padEnd(3, "0")) : NaN;
    };
    const start = time(value("start")), end = time(value("end"));
    if (!Number.isFinite(start) || !Number.isFinite(end)) continue;
    cues.push({ start, end, text: body.replace(/\{[^}]*\}/g, "").replace(/\\[Nn]/g, "\n").replace(/\\h/g, " ").trim() });
  }
  return cues.sort((a, b) => a.start - b.start);
}

/** Cues of an uploaded track, for B11. */
export const cuesOf = (format: "srt" | "ass", bytes: Uint8Array) => {
  const text = new TextDecoder().decode(bytes);
  return (format === "ass" ? parseAss(text) : parseSrt(text)).filter((c) => c.end >= c.start);
};

// ------------------------------------------------------------------ conversions
/** Drop the [Fonts] and [Graphics] sections: the player uses its bundled fonts only. */
function stripAttachments(text: string) {
  const out: string[] = [];
  let skip = false;
  for (const line of text.split(/(?<=\n)/)) {
    const header = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (header) skip = /^(Fonts|Graphics)$/i.test(header[1]!.trim());
    if (!skip) out.push(line);
  }
  return out.join("");
}

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
const entities = (text: string) =>
  text.replace(/&(#x?[0-9a-f]+|\w+);/gi, (all, name: string) => {
    if (name[0] === "#") {
      const code = name[1] === "x" || name[1] === "X" ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : all;
    }
    return ENTITIES[name.toLowerCase()] ?? all;
  });

const VTT_TIME = /^((?:\d+:)?\d{1,2}:\d{1,2}\.\d{1,3})\s+-->\s+((?:\d+:)?\d{1,2}:\d{1,2}\.\d{1,3})/;
const vttSeconds = (v: string) => {
  const parts = v.split(":");
  const [s, f] = parts.pop()!.split(".");
  const m = parts.pop() ?? "0", h = parts.pop() ?? "0";
  return seconds(h, m, s!, f!);
};
/** WebVTT cues; STYLE, REGION and NOTE blocks and cue settings are dropped, <b>/<i>/<u> kept. */
export function parseVtt(text: string): Cue[] {
  const cues: Cue[] = [];
  for (const block of text.replace(/\r\n?/g, "\n").split(/\n\s*\n/)) {
    const lines = block.split("\n");
    const at = lines.findIndex((line) => VTT_TIME.test(line.trim()));
    if (at < 0 || at > 1) continue;
    const m = VTT_TIME.exec(lines[at]!.trim())!;
    const body = lines
      .slice(at + 1)
      .join("\n")
      .replace(/<\d{1,2}:[\d:.]+>/g, "") // karaoke timestamps
      .replace(/<\/?(c|v|lang|ruby|rt)(\.[^\s>]*)?(\s[^>]*)?>/gi, "");
    cues.push({ start: vttSeconds(m[1]!), end: vttSeconds(m[2]!), text: entities(body) });
  }
  return cues;
}

/** SAMI: one cue list per class (language); `&nbsp;` lines end the previous cue. */
export function parseSami(text: string): { language: string | null; cues: Cue[] }[] {
  const names = new Map<string, string>();
  for (const m of text.matchAll(/\.([\w-]+)\s*\{([^}]*)\}/g)) {
    const name = /(?:^|;)\s*name\s*:\s*([^;]+)/i.exec(m[2]!)?.[1]?.trim();
    const lang = /(?:^|;)\s*lang\s*:\s*([^;]+)/i.exec(m[2]!)?.[1]?.trim();
    names.set(m[1]!.toLowerCase(), name || lang || m[1]!);
  }
  const body = text.replace(/<!--[\s\S]*?-->/g, "");
  const syncs = [...body.matchAll(/<SYNC\b[^>]*\bStart\s*=\s*"?(\d+)"?[^>]*>/gi)];
  const open = new Map<string, Cue | null>();
  const byClass = new Map<string, Cue[]>();
  syncs.forEach((sync, i) => {
    const start = Number(sync[1]) / 1000;
    const chunk = body.slice(sync.index! + sync[0].length, syncs[i + 1]?.index ?? body.length);
    const paragraphs = [...chunk.matchAll(/<P\b([^>]*)>([\s\S]*?)(?=<P\b|$)/gi)];
    const items = paragraphs.length ? paragraphs.map((p) => [/class\s*=\s*"?([\w-]+)/i.exec(p[1]!)?.[1] ?? "", p[2]!] as const) : [["", chunk] as const];
    for (const [klass, html] of items) {
      const key = klass.toLowerCase();
      const previous = open.get(key);
      if (previous) previous.end = start;
      const line = entities(html.replace(/<br\s*\/?>/gi, "\n").replace(/<[^>]+>/g, ""))
        .split("\n")
        .map((l) => l.replace(/\s+/g, " ").trim())
        .filter(Boolean)
        .join("\n");
      if (!line) {
        open.set(key, null);
        continue;
      }
      const cue = { start, end: start + 5, text: line };
      open.set(key, cue);
      if (!byClass.has(key)) byClass.set(key, []);
      byClass.get(key)!.push(cue);
    }
  });
  const result = [...byClass].map(([key, cues]) => ({ language: names.get(key) ?? (key || null), cues }));
  return result.length === 1 ? [{ language: null, cues: result[0]!.cues }] : result;
}

/** MicroDVD `{start}{end}text` frames; `|` breaks lines, `{y:i}`-style codes dropped. */
export function parseMicroDvd(text: string, videoFps: number | null) {
  const lines = text.replace(/\r\n?/g, "\n").split("\n").filter((l) => l.trim());
  const declared = /^\{1\}\{1\}(\d+(?:\.\d+)?)\s*$/.exec(lines[0]?.trim() ?? "");
  let fps = videoFps && videoFps > 0 ? videoFps : declared ? Number(declared[1]) : 0;
  const guessed = !(fps > 0);
  if (guessed) fps = 23.976;
  const cues: Cue[] = [];
  for (const line of declared ? lines.slice(1) : lines) {
    const m = /^\{(\d+)\}\{(\d*)\}(.*)$/.exec(line.trim());
    if (!m) continue;
    const start = Number(m[1]) / fps;
    const end = m[2] ? Number(m[2]) / fps : start + 3;
    cues.push({ start, end, text: m[3]!.replace(/\{[^}]*\}/g, "").split("|").join("\n") });
  }
  return { cues, guessed, fps };
}
