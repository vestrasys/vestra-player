import { renameSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { extensionOf } from "./media.ts";

/** C18: one entry of a playlist file. */
export type ListEntry = {
  kind: "file" | "url";
  source: string;
  /** Title given by the list file (`#EXTINF`, `TitleN`, `*title*`). */
  title: string | null;
  /** Seconds; null when unknown. */
  duration: number | null;
};

export const LIST_SIZE_LIMIT = 16 * 1024 * 1024;
export const LIST_ENTRY_LIMIT = 20_000;

const SCHEME = /^[a-z][a-z0-9+.-]*:\/\//i;

/** Text of a list file: `.m3u8` is UTF-8; others try strict UTF-8, then GB18030, then Latin-1. BOMs are dropped. */
export function decodeList(bytes: Uint8Array, ext: string) {
  if (ext === "m3u8") return new TextDecoder("utf-8").decode(bytes);
  for (const label of ["utf-8", "gb18030"]) {
    try {
      return new TextDecoder(label, { fatal: true }).decode(bytes);
    } catch {
      /* next */
    }
  }
  return new TextDecoder("latin1").decode(bytes);
}

/** An entry's location: `file://` URLs become paths, relative paths resolve against the list's folder. */
function locate(line: string, base: string): Pick<ListEntry, "kind" | "source"> | null {
  const text = line.trim();
  if (!text) return null;
  if (/^file:\/\//i.test(text)) {
    try {
      return { kind: "file", source: fileURLToPath(text) };
    } catch {
      return null;
    }
  }
  if (SCHEME.test(text)) return { kind: "url", source: text };
  return { kind: "file", source: resolve(base, text) };
}

const seconds = (value: string | undefined, scale = 1) => {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? number / scale : null;
};

/** Parse a list file's text; unreadable lines are counted in `skipped`. */
export function parseList(text: string, ext: string, base: string): { entries: ListEntry[]; skipped: number } {
  const lines = text.replace(/^﻿/, "").split(/\r\n|\r|\n/);
  const entries: ListEntry[] = [];
  let skipped = 0;
  const push = (line: string | undefined, title: string | null, duration: number | null) => {
    const where = line === undefined ? null : locate(line, base);
    if (where) entries.push({ ...where, title: title?.trim() || null, duration });
    else skipped++;
  };
  if (ext === "pls") {
    const fields = new Map<string, string>();
    for (const line of lines) {
      const at = line.indexOf("=");
      if (at > 0) fields.set(line.slice(0, at).trim().toLowerCase(), line.slice(at + 1));
    }
    const numbers = [...fields.keys()]
      .map((key) => /^file(\d+)$/.exec(key)?.[1])
      .filter((n): n is string => n !== undefined)
      .map(Number)
      .sort((a, b) => a - b);
    for (const n of numbers)
      push(fields.get(`file${n}`), fields.get(`title${n}`) ?? null, seconds(fields.get(`length${n}`)));
  } else if (ext === "dpl") {
    const items = new Map<number, { file?: string; title?: string; duration?: string }>();
    for (const line of lines) {
      const match = /^(\d+)\*(file|title|duration2)\*(.*)$/.exec(line);
      if (!match) continue;
      const item = items.get(Number(match[1])) ?? {};
      if (match[2] === "file") item.file = match[3];
      else if (match[2] === "title") item.title = match[3];
      else item.duration = match[3];
      items.set(Number(match[1]), item);
    }
    for (const [, item] of [...items].sort(([a], [b]) => a - b))
      push(item.file, item.title ?? null, seconds(item.duration, 1000));
  } else {
    let info: { title: string | null; duration: number | null } | null = null;
    for (const line of lines) {
      const text = line.trim();
      if (!text) continue;
      if (text.startsWith("#")) {
        const match = /^#EXTINF:\s*(-?[\d.]+)[^,]*,(.*)$/i.exec(text);
        if (match) info = { title: match[2]!, duration: seconds(match[1]) };
        continue;
      }
      push(text, info?.title ?? null, info?.duration ?? null);
      info = null;
    }
  }
  return { entries, skipped };
}

/** What a saved row needs. */
export type SavedRow = { kind: "file" | "url"; source: string; title: string; duration: number | null };

/** Write a list file the user chose atomically (temporary file, then rename). */
export function writeAtomic(path: string, data: string | Uint8Array) {
  const temporary = `${path}.tmp-${process.pid}`;
  writeFileSync(temporary, data);
  renameSync(temporary, path);
}

/** The list format for a chosen file name: `.m3u8`, `.m3u` or `.pls`; anything else gets `.m3u8` appended. */
export function savePath(path: string) {
  const ext = extensionOf(path);
  return ["m3u8", "m3u", "pls"].includes(ext) ? { path, ext } : { path: `${path}.m3u8`, ext: "m3u8" };
}

/**
 * Rows below the list's folder are written relative to it; others absolute; links verbatim.
 * Relative paths always use `/`, so a list saved on Windows still works when shared with
 * macOS or Linux (where `\` is a file name character); Windows players read `/` too.
 * Absolute paths stay native: they only make sense on the machine that wrote them.
 */
function written(row: SavedRow, folder: string) {
  if (row.kind === "url") return row.source;
  const rel = relative(folder, row.source);
  return rel && !rel.startsWith(`..${sep}`) && rel !== ".." && !isAbsolute(rel) ? rel.split(sep).join("/") : row.source;
}

/** The bytes of a list file at `path` (format from its extension, see `savePath`). */
export function formatList(rows: SavedRow[], path: string, ext: string): Uint8Array {
  const folder = dirname(path);
  const length = (row: SavedRow) => (row.duration !== null && row.duration > 0 ? Math.round(row.duration) : -1);
  const oneLine = (text: string) => text.replace(/[\r\n]+/g, " ");
  let text: string;
  if (ext === "pls") {
    const lines = ["[playlist]"];
    rows.forEach((row, i) => {
      lines.push(`File${i + 1}=${written(row, folder)}`, `Title${i + 1}=${oneLine(row.title)}`, `Length${i + 1}=${length(row)}`);
    });
    lines.push(`NumberOfEntries=${rows.length}`, "Version=2");
    text = lines.join("\n") + "\n";
  } else {
    const lines = ["#EXTM3U"];
    for (const row of rows) lines.push(`#EXTINF:${length(row)},${oneLine(row.title)}`, written(row, folder));
    text = lines.join("\n") + "\n";
  }
  const body = new TextEncoder().encode(text);
  if (ext !== "m3u") return body;
  // `.m3u` carries a BOM so Windows players read it as UTF-8.
  const bytes = new Uint8Array(body.length + 3);
  bytes.set([0xef, 0xbb, 0xbf]);
  bytes.set(body, 3);
  return bytes;
}

/** The list's name as shown in notices and as the default save name. */
export const listName = (path: string) => basename(path, extname(path));
