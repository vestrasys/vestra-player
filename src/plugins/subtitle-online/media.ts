// What a search sends about the video: the OpenSubtitles moviehash and title keywords.
import { open } from "node:fs/promises";
import { basename, extname } from "node:path";
import { inflateRawSync } from "node:zlib";

const CHUNK = 64 * 1024;

/**
 * OpenSubtitles moviehash: file size plus the first and last 64 KiB read as
 * little-endian 64-bit integers, summed modulo 2^64; 16 hex digits. `null` for
 * files under 128 KiB or unreadable ones.
 */
export async function movieHash(path: string): Promise<{ hash: string; size: number } | null> {
  const file = await open(path, "r").catch(() => null);
  if (!file) return null;
  try {
    const { size } = await file.stat();
    if (size < 2 * CHUNK) return null;
    const head = Buffer.alloc(CHUNK), tail = Buffer.alloc(CHUNK);
    await file.read(head, 0, CHUNK, 0);
    await file.read(tail, 0, CHUNK, size - CHUNK);
    let sum = BigInt(size);
    for (const buffer of [head, tail])
      for (let i = 0; i < CHUNK; i += 8) sum = (sum + buffer.readBigUInt64LE(i)) & 0xffffffffffffffffn;
    return { hash: sum.toString(16).padStart(16, "0"), size };
  } finally {
    await file.close();
  }
}

/** Tokens that start the technical tail of a release name (PotPlayer `%SS`). */
const TECHNICAL =
  /^(\d{3,4}p|4k|uhd|x26[45]|h\.?26[45]|hevc|avc|xvid|divx|blu-?ray|bdrip|brrip|web-?dl|web-?rip|web|hdtv|hdrip|dvd(rip|scr)?|remux|hdr(10)?|dv|10bit|8bit|aac(2\.0)?|ac3|dts(-hd)?|truehd|atmos|ddp?(5\.1)?|5\.1|7\.1|proper|repack|extended|unrated|imax|multi|complete|chs|cht|chi|eng)$/i;

/**
 * Search keywords from a file name: no extension, dots and underscores as
 * spaces, bracketed release groups removed, cut before the first resolution,
 * codec or source token. Keeps the year and SxxEyy.
 */
export function keywords(name: string) {
  const stem = basename(name, extname(name))
    .replace(/\[[^\]]*\]|\([^)]*(?:\d{3,4}p|x26[45]|web|bluray)[^)]*\)/gi, " ")
    .replace(/[._]+/g, " ");
  const words: string[] = [];
  for (const word of stem.split(/\s+/).filter(Boolean)) {
    if (TECHNICAL.test(word.replace(/-\w+$/, "")) || TECHNICAL.test(word)) break;
    words.push(word);
  }
  const text = words.join(" ").replace(/-\s*$/, "").trim();
  return text || stem.trim();
}

/** Title keywords for a network source: the last path segment of the URL. */
export function urlTitle(source: string) {
  try {
    const url = new URL(source);
    const last = decodeURIComponent(url.pathname.split("/").filter(Boolean).pop() ?? url.hostname);
    return keywords(last);
  } catch {
    return keywords(source);
  }
}

// ------------------------------------------------------------------ zip
export const SUBTITLE_EXTENSIONS = /\.(srt|ass|ssa|vtt|smi|sami|sub)$/i;
export type Archive = "zip" | "rar" | "7z" | null;
export function archiveKind(bytes: Uint8Array): Archive {
  if (bytes[0] === 0x50 && bytes[1] === 0x4b && bytes[2] === 0x03 && bytes[3] === 0x04) return "zip";
  if (bytes[0] === 0x52 && bytes[1] === 0x61 && bytes[2] === 0x72 && bytes[3] === 0x21) return "rar";
  if (bytes[0] === 0x37 && bytes[1] === 0x7a && bytes[2] === 0xbc && bytes[3] === 0xaf) return "7z";
  return null;
}

/**
 * The subtitle files in a zip archive (stored or deflated entries, read from
 * the central directory). Names are decoded as UTF-8 when flagged, else GB18030
 * (Chinese archives usually are). At most 64 files of 8 MiB each.
 */
export function unzipSubtitles(bytes: Uint8Array): { name: string; bytes: Buffer }[] {
  const data = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let end = -1;
  for (let i = data.length - 22; i >= Math.max(0, data.length - 65557); i--)
    if (data.readUInt32LE(i) === 0x06054b50) {
      end = i;
      break;
    }
  if (end < 0) throw new Error("压缩包已损坏");
  const count = data.readUInt16LE(end + 10);
  let at = data.readUInt32LE(end + 16);
  const files: { name: string; bytes: Buffer }[] = [];
  for (let n = 0; n < count && at + 46 <= data.length; n++) {
    if (data.readUInt32LE(at) !== 0x02014b50) throw new Error("压缩包已损坏");
    const flags = data.readUInt16LE(at + 8), method = data.readUInt16LE(at + 10);
    const compressed = data.readUInt32LE(at + 20), size = data.readUInt32LE(at + 24);
    const nameLength = data.readUInt16LE(at + 28), extra = data.readUInt16LE(at + 30), comment = data.readUInt16LE(at + 32);
    const local = data.readUInt32LE(at + 42);
    const rawName = data.subarray(at + 46, at + 46 + nameLength);
    at += 46 + nameLength + extra + comment;
    const name = new TextDecoder(flags & 0x800 ? "utf-8" : "gb18030").decode(rawName);
    if (name.endsWith("/") || !SUBTITLE_EXTENSIONS.test(name) || /(^|\/)__MACOSX\//.test(name)) continue;
    if (size > 8 * 1024 * 1024 || flags & 1) continue; // too large, or encrypted
    if (data.readUInt32LE(local) !== 0x04034b50) continue;
    const start = local + 30 + data.readUInt16LE(local + 26) + data.readUInt16LE(local + 28);
    const raw = data.subarray(start, start + compressed);
    let content: Buffer;
    if (method === 0) content = Buffer.from(raw);
    else if (method === 8) content = inflateRawSync(raw, { maxOutputLength: 8 * 1024 * 1024 });
    else continue;
    files.push({ name: name.split("/").pop()!, bytes: content });
    if (files.length >= 64) break;
  }
  return files;
}
