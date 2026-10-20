import { readdir, realpath, stat } from "node:fs/promises";
import { basename, dirname, extname, join, sep } from "node:path";

/** Appendix B.5 media extensions (case-insensitive). */
export const VIDEO = new Set(
  "mp4 m4v mkv webm mov avi wmv asf flv f4v ts m2ts mts mpg mpeg vob ogv 3gp rm rmvb".split(" "),
);
export const AUDIO = new Set(
  "mp3 m4a aac flac wav ogg oga opus wma ape alac aif aiff dsf dff mka wv tta ac3 dts".split(" "),
);
export const LIST_FILES = new Set(["m3u", "m3u8", "pls", "dpl"]);

export const extensionOf = (path: string) => extname(path).slice(1).toLowerCase();
export const mediaKind = (path: string): "video" | "audio" | null => {
  const ext = extensionOf(path);
  return VIDEO.has(ext) ? "video" : AUDIO.has(ext) ? "audio" : null;
};
export const isMedia = (path: string) => mediaKind(path) !== null;

export const collator = new Intl.Collator("zh", { numeric: true, sensitivity: "base" });

/** "By folder": parent path segment by segment (a folder's own files before its subfolders), then the name. */
export function compareByFolder(a: string, b: string) {
  const left = dirname(a).split(sep),
    right = dirname(b).split(sep);
  for (let i = 0; i < Math.min(left.length, right.length); i++) {
    const order = collator.compare(left[i]!, right[i]!);
    if (order) return order;
  }
  return left.length - right.length || collator.compare(basename(a), basename(b));
}

/** macOS packages are folders to Node but single items to the user. */
const PACKAGES = new Set(
  "app bundle framework plugin kext photoslibrary fcpbundle imovielibrary musiclibrary tvlibrary rtfd pkg xcodeproj xcworkspace"
    .split(" "),
);
const isPackage = (name: string) => PACKAGES.has(extensionOf(name));

export const SCAN_DEPTH = 8;
export const SCAN_LIMIT = 5000;
const YIELD_EVERY = 200;

export type ScannedFile = { path: string; key: string };
export type Scan = { files: ScannedFile[]; unreadable: number; truncated: boolean };

const yieldLoop = () => new Promise<void>((resolve) => setImmediate(resolve));

/**
 * Media files under `root` (recursive, depth ≤ 8, at most `limit`), sorted "by folder".
 * Follows symlinks but visits each real directory and file once. Throws when `root`
 * itself cannot be read; unreadable subfolders are counted.
 */
export async function scanFolder(
  root: string,
  options: { signal?: AbortSignal; limit?: number; depth?: number } = {},
): Promise<Scan> {
  const limit = options.limit ?? SCAN_LIMIT,
    maxDepth = options.depth ?? SCAN_DEPTH;
  const seenDirs = new Set<string>(),
    seenFiles = new Set<string>();
  const files: ScannedFile[] = [];
  let unreadable = 0,
    truncated = false,
    visited = 0;
  const check = async () => {
    if (options.signal?.aborted) throw Object.assign(new Error("aborted"), { name: "AbortError" });
    if (++visited % YIELD_EVERY === 0) await yieldLoop();
  };

  async function walk(dir: string, depth: number, top: boolean): Promise<void> {
    let real: string;
    let entries: import("node:fs").Dirent[];
    try {
      real = await realpath(dir);
      if (seenDirs.has(real)) return;
      seenDirs.add(real);
      entries = await readdir(dir, { withFileTypes: true });
    } catch (error) {
      if (top) throw error;
      unreadable++;
      return;
    }
    const subdirs: string[] = [];
    for (const entry of entries) {
      await check();
      if (entry.name.startsWith(".")) continue;
      const path = join(dir, entry.name);
      let isDir = entry.isDirectory(),
        isFile = entry.isFile();
      if (entry.isSymbolicLink()) {
        try {
          const target = await stat(path);
          isDir = target.isDirectory();
          isFile = target.isFile();
        } catch {
          continue;
        }
      }
      if (isDir) {
        if (!isPackage(entry.name) && depth < maxDepth) subdirs.push(path);
      } else if (isFile && isMedia(entry.name)) {
        let key = path;
        try {
          key = await realpath(path);
        } catch {
          continue;
        }
        if (seenFiles.has(key)) continue;
        if (files.length >= limit) {
          truncated = true;
          continue;
        }
        seenFiles.add(key);
        files.push({ path, key });
      }
    }
    for (const sub of subdirs.sort((a, b) => collator.compare(basename(a), basename(b))))
      await walk(sub, depth + 1, false);
  }

  await walk(root, 0, true);
  files.sort((a, b) => compareByFolder(a.path, b.path));
  return { files, unreadable, truncated };
}

/** Why a folder could not be read, as the user should read it. */
export function folderError(error: unknown) {
  const code = (error as NodeJS.ErrnoException).code;
  if (code === "EPERM") return "未获准访问此位置（系统设置 ▶ 隐私与安全性 ▶ 文件与文件夹）";
  if (code === "EACCES") return "没有读取权限";
  if (code === "ENOENT") return "文件夹不存在";
  if (code === "ENOTDIR") return "不是文件夹";
  return error instanceof Error ? error.message : String(error);
}

/**
 * A13 "similar" name: no extension, lower case, digit runs as `#`, bracketed parts
 * removed, `._-` and white space folded to one space.
 */
export function similarName(path: string) {
  return basename(path, extname(path))
    .toLowerCase()
    .replace(/\[[^\]]*\]|\([^)]*\)|【[^】]*】/g, " ")
    .replace(/\d+/g, "#")
    .replace(/[._\-\s]+/g, " ")
    .trim();
}
export function isSimilar(a: string, b: string) {
  if (mediaKind(a) !== mediaKind(b)) return false;
  const x = similarName(a),
    y = similarName(b);
  if (x === y) return true;
  let common = 0;
  while (common < x.length && common < y.length && x[common] === y[common]) common++;
  return common >= 4 && common >= Math.min(x.length, y.length) * 0.5;
}

export const SIBLING_LIMIT = 1000;

/** A13: media files next to `path` (not recursive), naturally sorted; `path` itself always included. */
export async function siblings(path: string, mode: "similar" | "folder"): Promise<string[]> {
  const dir = dirname(path);
  const entries = await readdir(dir, { withFileTypes: true });
  const own = basename(path);
  const found: string[] = [];
  for (const entry of entries) {
    if (entry.name.startsWith(".") && entry.name !== own) continue;
    const full = join(dir, entry.name);
    if (entry.name !== own) {
      if (!isMedia(entry.name)) continue;
      if (!(entry.isFile() || entry.isSymbolicLink())) continue;
      if (mode === "similar" && !isSimilar(own, entry.name)) continue;
    }
    found.push(full);
  }
  if (!found.includes(path)) found.push(path);
  found.sort((a, b) => collator.compare(basename(a), basename(b)));
  if (found.length <= SIBLING_LIMIT) return found;
  // Keep the opened file within the limit.
  const at = found.indexOf(path);
  const start = Math.max(0, Math.min(at - Math.floor(SIBLING_LIMIT / 2), found.length - SIBLING_LIMIT));
  return found.slice(start, start + SIBLING_LIMIT);
}
