import { execFile } from "node:child_process";
import { readFile, stat } from "node:fs/promises";
import { basename, dirname } from "node:path";
import { decodeList, LIST_ENTRY_LIMIT, LIST_SIZE_LIMIT, parseList } from "./listfile.ts";
import { AUDIO, extensionOf, folderError, isMedia, isPackage, LIST_FILES, scanFolder, SCAN_LIMIT, VIDEO } from "./media.ts";
import type { Entry } from "./queue.ts";

/**
 * A15: what playlist takes from a drop on the video area (`drops.provide`):
 * media files, list files (C18, plus Safari/Chrome `.webloc` links) and
 * folders (the player's reserved name `"folder"`). Subtitles, images and
 * anything else are left to other plugins.
 */
export const DROP_EXTENSIONS = [...VIDEO, ...AUDIO, ...LIST_FILES, "webloc", "folder"];

/** The list area of `ui/sidebar.slint` in panel coordinates: rows start below the 31px tab strip. */
export const LIST_TOP = 31;
export const ROW_HEIGHT = 20;

/**
 * A15 drop on the list: the shown row under `y` (panel coordinates, the list
 * scrolled by `scroll`), so the drop goes before it; `null` on blank space,
 * the tab strip or the bottom bar (`listHeight` tall list area) = the end.
 */
export function dropRow(y: number, scroll: number, listHeight: number, shown: number): number | null {
  const inList = y - LIST_TOP;
  if (!Number.isFinite(inList) || inList < 0 || inList >= listHeight) return null;
  const row = Math.floor((inList + Math.max(0, scroll)) / ROW_HEIGHT);
  return row < shown ? row : null;
}

export type Expanded = {
  entries: Entry[];
  /** Subfolders that could not be read, folders cut at `SCAN_LIMIT`. */
  unreadable: number;
  truncated: boolean;
  /** Dropped items playlist does not take, and unusable list lines. */
  skipped: number;
  /** `<name>：<reason>` for folders and list files that could not be read. */
  failed: string[];
};

const HTTP = /^https?:\/\//i;

/** The `URL` of a `.webloc` (an XML or binary property list); `null` when it has none. */
export async function weblocUrl(path: string): Promise<string | null> {
  const bytes = await readFile(path);
  let text = bytes.toString("utf8");
  if (text.startsWith("bplist")) {
    text = await new Promise<string>((resolve, reject) =>
      execFile("/usr/bin/plutil", ["-convert", "xml1", "-o", "-", path], { timeout: 5000 }, (error, stdout) =>
        error ? reject(error) : resolve(stdout),
      ),
    );
  }
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
 * A15: the dropped paths as list entries, in Finder order, each expanded:
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
        out.failed.push(`${name}：${folderError(error)}`);
      }
      continue;
    }
    const ext = extensionOf(path);
    if (LIST_FILES.has(ext)) {
      try {
        if ((info?.size ?? 0) > LIST_SIZE_LIMIT) throw new Error("文件超过 16 MB");
        const parsed = parseList(decodeList(await readFile(path), ext), ext, dirname(path));
        const entries = parsed.entries.slice(0, LIST_ENTRY_LIMIT);
        out.skipped += parsed.skipped + parsed.entries.length - entries.length;
        for (const entry of entries)
          out.entries.push({
            kind: entry.kind,
            source: entry.source,
            named: entry.title,
            duration: entry.duration,
            missing: "文件不存在",
          });
      } catch (error) {
        out.failed.push(`${name}：${error instanceof Error ? error.message : String(error)}`);
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

/** The extra part of the notice after a drop (after `已添加 N 项…`). */
export function dropExtra(expanded: Expanded) {
  return [
    expanded.skipped ? `跳过 ${expanded.skipped} 个不支持的项目` : "",
    expanded.unreadable ? `${expanded.unreadable} 个子文件夹无法读取` : "",
    expanded.truncated ? `已达上限 ${SCAN_LIMIT}` : "",
    ...expanded.failed.map((text) => `无法读取 ${text}`),
  ]
    .filter(Boolean)
    .join("，");
}
