import { realpathSync, statSync } from "node:fs";

/** Seconds to one decimal (B21 bookmark times, B23 A/B points). */
export const round1 = (seconds: number) => Math.round(seconds * 10) / 10;

const two = (n: number) => String(n).padStart(2, "0");

/** Menu and OSD time: `01:23`, `01:23:45` from one hour on (whole seconds). */
export function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds + 1e-6));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h ? `${two(h)}:${two(m)}:${two(s)}` : `${two(m)}:${two(s)}`;
}

/** B23 OSD and the list's time column: `m:ss.d`, `h:mm:ss.d` from one hour on. */
export function precise(seconds: number, hours = false): string {
  const tenths = Math.max(0, Math.round(seconds * 10));
  const d = tenths % 10;
  const total = Math.floor(tenths / 10);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h || hours ? `${h}:${two(m)}:${two(s)}.${d}` : `${m}:${two(s)}.${d}`;
}

/**
 * The list's time input: `h:mm:ss.d`, `m:ss.d` or plain seconds (`83.4`);
 * minutes and seconds after the first field are below 60. `null` when not a
 * time.
 */
export function parseTime(text: string): number | null {
  const parts = text.trim().replace(/：/g, ":").split(":");
  if (parts.length > 3 || parts.some((part) => !/^\d+(\.\d+)?$/.test(part))) return null;
  if (parts.slice(0, -1).some((part) => part.includes("."))) return null;
  const numbers = parts.map(Number);
  if (numbers.slice(1).some((n) => n >= 60)) return null;
  const seconds = numbers.reduce((sum, n) => sum * 60 + n, 0);
  return Number.isFinite(seconds) ? round1(seconds) : null;
}

const encoder = new TextEncoder();
const bytes = (text: string) => encoder.encode(text).length;

/**
 * Menu text is at most 80 UTF-8 bytes per path segment, no control
 * characters, no surrounding whitespace (host `commands.rs`): clean `text`
 * and shorten it with `…` to fit `max` bytes.
 */
export function fit(text: string, max: number): string {
  const clean = text.replace(/[\p{Cc}\s]+/gu, " ").trim();
  if (bytes(clean) <= max) return clean;
  const chars = [...clean];
  while (chars.length && bytes(chars.join("") + "…") > max) chars.pop();
  return `${chars.join("").trimEnd()}…`;
}

/** Fit `prefix + text + suffix` into `max` bytes by shortening `text`. */
export function line(prefix: string, text: string, suffix: string, max = 80): string {
  return `${prefix}${fit(text, Math.max(3, max - bytes(prefix) - bytes(suffix)))}${suffix}`;
}

/** A source is a local file when it is an absolute path; anything else is a URL. */
export const isLocal = (source: string) => source.startsWith("/");

/** B21 media identity: a local file by its real path (the path itself when that fails), a URL as is. */
export function mediaKey(source: string): string {
  if (!isLocal(source)) return source;
  try {
    return realpathSync.native(source);
  } catch {
    return source;
  }
}

/** The byte size of a local file; `null` for URLs and unreadable files. */
export function fileSize(source: string): number | null {
  if (!isLocal(source)) return null;
  try {
    return statSync(source).size;
  } catch {
    return null;
  }
}
