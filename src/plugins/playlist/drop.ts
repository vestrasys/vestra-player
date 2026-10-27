import { readFile, stat } from "node:fs/promises";
import { basename, dirname } from "node:path";
import { decodeList, LIST_ENTRY_LIMIT, LIST_SIZE_LIMIT, parseList } from "./listfile.ts";
import { AUDIO, extensionOf, folderError, isMedia, isPackage, LIST_FILES, scanFolder, VIDEO } from "./media.ts";
import type { NoticeText, ReadFailure, Texts } from "./notices.ts";
import { type Entry, type Problems, problemClauses } from "./queue.ts";

/**
 * A15: what playlist takes from a drop on the video area (`drops.provide`):
 * media files, list files (C18, plus Safari/Chrome `.webloc` links) and
 * folders (the player's reserved name `"folder"`). Subtitles, images and
 * anything else are left to other plugins.
 */
export const DROP_EXTENSIONS = [...VIDEO, ...AUDIO, ...LIST_FILES, "webloc", "folder"];

/**
 * The top of the list area of `ui/sidebar.slint` in panel coordinates while
 * the panel draws its own 31px tab strip; with the player's tab strip
 * (`PlaylistBridge.tabbed`) the list starts at 0. The list reports its actual
 * top through `list-area`; this is the value before the first report.
 */
export const LIST_TOP = 31;
/** A row of `ui/sidebar.slint`: one line, or (0.1.11) two lines beside a 96×54 thumbnail. */
export const ROW_HEIGHT = 20;
export const THUMB_ROW_HEIGHT = 62;

/**
 * A15 drop on the list: the shown row under `y` (panel coordinates, the list
 * scrolled by `scroll`, starting at `top`, rows `row` px high), so the drop goes
 * before it; `null` on blank space, the tab strip or the bottom bar
 * (`listHeight` tall list area) = the end.
 */
export function dropRow(
  y: number,
  scroll: number,
  listHeight: number,
  shown: number,
  top = LIST_TOP,
  row = ROW_HEIGHT,
): number | null {
  const inList = y - top;
  if (!Number.isFinite(inList) || inList < 0 || inList >= listHeight) return null;
  const index = Math.floor((inList + Math.max(0, scroll)) / row);
  return index < shown ? index : null;
}

export type Expanded = {
  entries: Entry[];
  /** Subfolders that could not be read, folders cut at `SCAN_LIMIT`. */
  unreadable: number;
  truncated: boolean;
  /** Dropped items playlist does not take, and unusable list lines. */
  skipped: number;
  /** Folders and list files that could not be read. */
  failed: { name: string; failure: ReadFailure }[];
};

const HTTP = /^https?:\/\//i;

/**
 * The `URL` string of a binary property list (`bplist00`) whose top object is
 * a dictionary; `null` when it has none or the file is malformed. A `.webloc`
 * holds just that one string, so only dictionaries, ASCII and UTF-16 strings
 * and the integers giving long lengths are read.
 */
export function binaryPlistUrl(bytes: Uint8Array): string | null {
  const data = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (data.length < 40 || data.toString("latin1", 0, 8) !== "bplist00") return null;
  const trailer = data.length - 32;
  const offsetSize = data[trailer + 6]!;
  const refSize = data[trailer + 7]!;
  const count = Number(data.readBigUInt64BE(trailer + 8));
  const top = Number(data.readBigUInt64BE(trailer + 16));
  const table = Number(data.readBigUInt64BE(trailer + 24));
  const sized = (n: number) => n >= 1 && n <= 8;
  if (!sized(offsetSize) || !sized(refSize) || top >= count || table + count * offsetSize > trailer) return null;
  const span = (at: number, size: number) => {
    if (at < 8 || at + size > trailer) throw new RangeError("outside the object area");
    return data.subarray(at, at + size);
  };
  const uint = (at: number, size: number) => span(at, size).reduce((value, byte) => value * 256 + byte, 0);
  const offset = (ref: number) => {
    if (ref >= count) throw new RangeError("bad object reference");
    return uint(table + ref * offsetSize, offsetSize);
  };
  /** Marker high nibble, element count and where the elements start. */
  const header = (at: number) => {
    const marker = uint(at, 1);
    let length = marker & 0x0f;
    let start = at + 1;
    if (length === 0x0f) {
      const int = uint(start, 1);
      if (int >> 4 !== 0x1) throw new RangeError("bad length");
      const size = 1 << (int & 0x0f);
      length = uint(start + 1, size);
      start += 1 + size;
    }
    return { type: marker >> 4, length, start };
  };
  const string = (ref: number) => {
    const { type, length, start } = header(offset(ref));
    if (type === 0x5) return span(start, length).toString("latin1");
    if (type === 0x6) return Buffer.from(span(start, length * 2)).swap16().toString("utf16le");
    return null;
  };
  try {
    const dict = header(offset(top));
    if (dict.type !== 0xd) return null;
    for (let i = 0; i < dict.length; i++)
      if (string(uint(dict.start + i * refSize, refSize)) === "URL")
        return string(uint(dict.start + (dict.length + i) * refSize, refSize));
  } catch {
    // malformed
  }
  return null;
}

/** The `URL` of a `.webloc` (an XML or binary property list); `null` when it has none. */
export async function weblocUrl(path: string): Promise<string | null> {
  const bytes = await readFile(path);
  if (bytes.toString("latin1", 0, 6) === "bplist") {
    const url = binaryPlistUrl(bytes)?.trim();
    return url && HTTP.test(url) ? url : null;
  }
  const text = bytes.toString("utf8");
  const match = /<key>URL<\/key>\s*<string>([^<]*)<\/string>/.exec(text);
  if (!match) return null;
  const url = match[1]!
    .trim()
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'");
  return HTTP.test(url) ? url : null;
}

/**
 * A15: the dropped paths as list entries, in the file manager's order, each expanded:
 * a folder as A03 (recursive, at most 5000 files, sorted by folder), a list
 * file as C18, a `.webloc` as its HTTP(S) link, a media file as itself.
 */
export async function expandDrop(paths: readonly string[], options: { signal?: AbortSignal } = {}): Promise<Expanded> {
  const out: Expanded = { entries: [], unreadable: 0, truncated: false, skipped: 0, failed: [] };
  for (const path of paths) {
    const name = basename(path);
    const info = await stat(path).catch(() => null);
    if (info?.isDirectory()) {
      if (isPackage(name)) {
        out.skipped++;
        continue;
      }
      try {
        const scan = await scanFolder(path, { signal: options.signal });
        out.unreadable += scan.unreadable;
        out.truncated ||= scan.truncated;
        for (const file of scan.files) out.entries.push({ kind: "file", source: file.path, key: file.key });
      } catch (error) {
        if (options.signal?.aborted) throw error;
        out.failed.push({ name, failure: folderError(error) });
      }
      continue;
    }
    const ext = extensionOf(path);
    if (LIST_FILES.has(ext)) {
      if ((info?.size ?? 0) > LIST_SIZE_LIMIT) {
        out.failed.push({ name, failure: { reason: "large" } });
        continue;
      }
      try {
        const parsed = parseList(decodeList(await readFile(path), ext), ext, dirname(path));
        const entries = parsed.entries.slice(0, LIST_ENTRY_LIMIT);
        out.skipped += parsed.skipped + parsed.entries.length - entries.length;
        for (const entry of entries)
          out.entries.push({
            kind: entry.kind,
            source: entry.source,
            named: entry.title,
            duration: entry.duration,
            missing: true,
          });
      } catch (error) {
        out.failed.push({ name, failure: { reason: "message", message: error instanceof Error ? error.message : String(error) } });
      }
    } else if (ext === "webloc") {
      const url = await weblocUrl(path).catch(() => null);
      if (url) out.entries.push({ kind: "url", source: url });
      else out.skipped++;
    } else if (isMedia(path)) out.entries.push({ kind: "file", source: path });
    else out.skipped++;
  }
  return out;
}

/** What a drop could not take, for the notice after adding. */
export const dropProblems = (expanded: Expanded): Problems => ({
  skipped: expanded.skipped,
  unreadable: expanded.unreadable,
  truncated: expanded.truncated,
  failed: expanded.failed,
});

/** The notice when a drop brought nothing to add: every problem, as after adding. */
export function nothingToAdd(texts: Texts, expanded: Expanded): NoticeText {
  const clauses = problemClauses(texts, dropProblems(expanded));
  return clauses.length ? texts.text("Nothing to add: {}", [texts.list(clauses, "clauses")]) : "Nothing to add";
}
