import { readdir, realpath, stat } from "node:fs/promises";
import { basename, dirname, extname, join, sep } from "node:path";
import type { ReadFailure } from "./notices.ts";

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
export const isPackage = (name: string) => PACKAGES.has(extensionOf(name));

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
        // A plain file's realpath is its name in the folder's realpath; only links are resolved
        // (one realpath per file made large folders slow on Windows).
        let key = join(real, entry.name);
        if (entry.isSymbolicLink())
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
export function folderError(error: unknown): ReadFailure {
  const code = (error as NodeJS.ErrnoException).code;
  if (code === "EPERM") return { reason: "permission" };
  if (code === "EACCES") return { reason: "access" };
  if (code === "ENOENT") return { reason: "missing" };
  if (code === "ENOTDIR") return { reason: "not-folder" };
  return { reason: "message", message: error instanceof Error ? error.message : String(error) };
}

// ------------------------------------------------------------------ A13 / IA-PL-01 similar files
// The rule is IA-PL-01 「同类文件规则」 of the interaction spec. The constants and
// the integer divisions are part of the spec and must not be tuned.

/** `T1`: the separators the comparison ignores (12 characters). */
const T1 = new Set(" _-.(){}[]'\"");
/** ASCII 0–9 only: full-width and Chinese digits are not digits. */
const isDigit = (c: string | undefined) => c !== undefined && c >= "0" && c <= "9";
const half = (s: string) => Math.floor(s.length / 2);
const third = (s: string) => Math.floor(s.length / 3);
const stripT1 = (s: string) => [...s].filter((c) => !T1.has(c)).join("");

/** Of the first `n` characters: 100 when any is not a digit, else how many differ. */
export function digitDiff(x: string, y: string, n: number) {
  let differ = 0;
  for (let i = 0; i < n; i++) {
    if (!isDigit(x[i]) || !isDigit(y[i])) return 100;
    if (x[i] !== y[i]) differ++;
  }
  return differ;
}

/**
 * Without `T1`: within the shorter one's first two thirds, at least half the
 * longer one's length matches; a mismatch of two ASCII characters where one is a
 * digit stops the count, other mismatches are skipped.
 */
export function loosePrefix(x: string, y: string) {
  const a = stripT1(x),
    b = stripT1(y);
  const longest = Math.max(a.length, b.length);
  const n = Math.floor((Math.min(a.length, b.length) * 2) / 3);
  let match = 0;
  for (let i = 0; i < n; i++) {
    const p = a[i]!,
      q = b[i]!;
    if (p === q) match++;
    else if (p.charCodeAt(0) < 0x80 && q.charCodeAt(0) < 0x80 && (isDigit(p) || isDigit(q))) break;
  }
  return Math.floor(longest / 2) <= match;
}

/** Wildcard `*` (a digit after `mask`) compare; see similar-files.md §5 `maskEq`. */
export function maskEq(x: string, y: string, n: number, prefix: boolean) {
  if (!prefix && x.length !== y.length) return false;
  const length = Math.min(x.length, y.length);
  for (let i = 0; i < length; i++) {
    if (x[i] === y[i]) {
      if (prefix && i >= n) return true;
      continue;
    }
    if (x[i] === "*" || y[i] === "*") return false;
    return i >= n;
  }
  return true;
}

/** `T1` characters to `?`, digits to `*`; same length. */
export const mask = (s: string) => s.replace(/[\s\S]/g, (c) => (T1.has(c) ? "?" : isDigit(c) ? "*" : c));

/** Only A–Z are lowered (as in the C locale). */
const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());
/** What `isSimilar` compares: the lowered name without extension plus `.tmp` (counted in every length). */
export const similarKey = (stem: string) => `${asciiLower(stem)}.tmp`;

/**
 * The rule's four steps S1–S4 on two `similarKey`s (`audio` adds the looser audio
 * rule): the first step that finds them similar, `null` when none does.
 */
export type SimilarStep = "S1" | "S2" | "S3 equal" | "S3 prefix" | "S4 audio" | "S4 mask";
export function similarStep(a: string, b: string, audio: boolean): SimilarStep | null {
  if (Math.min(half(a), half(b)) >= 5) {
    if (loosePrefix(a, b)) return "S1";
    if (digitDiff(a, b, Math.min(third(a), third(b))) <= 2) return "S1";
  }
  const a2 = stripT1(a),
    b2 = stripT1(b);
  if (Math.min(half(a2), half(b2)) >= 5 && digitDiff(a2, b2, Math.min(third(a2), third(b2))) <= 2) return "S2";
  const a3 = a2.replace(/[0-9]/g, ""),
    b3 = b2.replace(/[0-9]/g, "");
  if (a3 === b3) return "S3 equal";
  if (Math.min(half(a3), half(b3)) >= 5 && maskEq(a3, b3, Math.min(third(a3), third(b3), 5), true)) return "S3 prefix";
  const am = mask(a),
    bm = mask(b);
  if (audio && (maskEq(am, bm, 2, false) || maskEq(am, bm, 3, true))) return "S4 audio";
  if (Math.min(half(am), half(bm)) < 5) return null;
  return maskEq(am, bm, Math.min(third(am), third(bm), 5), true) ? "S4 mask" : null;
}
export const isSimilar = (a: string, b: string, audio: boolean) => similarStep(a, b, audio) !== null;

/** The opened file's group: audio first, then video (whole extension, Appendix B.5); else none. */
const groupOf = (ext: string): "audio" | "video" | null => (AUDIO.has(ext) ? "audio" : VIDEO.has(ext) ? "video" : null);
/** 「相似」 needs a space, `-`, `_` or ASCII digit in the name, and two characters at least. */
export const canHaveSimilar = (stem: string) => stem.length >= 2 && /[ \-_0-9]/.test(stem);

const stemOf = (name: string) => basename(name, extname(name));

/**
 * A13 / IA-PL-01: the files of `path`'s folder (not recursive) that are
 * opened with it: the same group (video with video, audio with audio), and in
 * `similar` mode those `isSimilar` to it; `path` included, naturally sorted.
 * Names starting with `.` are skipped (macOS `._*` files; Vestra's deviation).
 * More than `SCAN_LIMIT` matches keep the `SCAN_LIMIT` around `path` (`truncated`).
 */
export async function siblings(path: string, mode: "similar" | "folder"): Promise<{ paths: string[]; truncated: boolean }> {
  const own = basename(path);
  const group = groupOf(extensionOf(own));
  const alone = { paths: [path], truncated: false };
  if (!group) return alone;
  const stem = stemOf(own);
  if (mode === "similar" && !canHaveSimilar(stem)) return alone;
  const dir = dirname(path);
  const key = similarKey(stem);
  const found = [path];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".") || entry.name === own) continue;
    if (groupOf(extensionOf(entry.name)) !== group) continue;
    const full = join(dir, entry.name);
    if (entry.isSymbolicLink()) {
      if (!(await stat(full).catch(() => null))?.isFile()) continue;
    } else if (!entry.isFile()) continue;
    if (mode === "similar" && !isSimilar(key, similarKey(stemOf(entry.name)), group === "audio")) continue;
    found.push(full);
  }
  found.sort((a, b) => collator.compare(basename(a), basename(b)));
  if (found.length <= SCAN_LIMIT) return { paths: found, truncated: false };
  const at = found.indexOf(path);
  const start = Math.max(0, Math.min(at - Math.floor(SCAN_LIMIT / 2), found.length - SCAN_LIMIT));
  return { paths: found.slice(start, start + SCAN_LIMIT), truncated: true };
}
