import { realpathSync, statSync } from "node:fs";
import { basename, extname, isAbsolute, posix, win32 } from "node:path";
import { fileURLToPath } from "node:url";

/*
 * Sources on each OS (the host hands over what was opened, as it was given):
 *
 * | source                         | macOS                       | Windows                         |
 * | ------------------------------ | --------------------------- | ------------------------------- |
 * | /Users/x/b.mkv                 | file                        | file (on the current drive)     |
 * | C:\x\b.mkv, C:/x/b.mkv         | link, shown as b.mkv        | file                            |
 * | \\server\share\b.mkv           | link, shown as b.mkv        | file                            |
 * | file:///…/b.mkv                | file (the path it names)    | file (the path it names)        |
 * | file://server/share/b.mkv      | link (no such path)         | file (\\server\share\b.mkv)     |
 * | https://…, rtsp://…            | link                        | link                            |
 *
 * A file is keyed by its real path, so a path and a `file:` URL of one file are
 * one record (and on Windows `C:/x` and `c:\x` too, while the file exists); a
 * link by its text without the `#` fragment. A file shows its name, a link
 * `host › last path segment`.
 */

/**
 * The local path a source names: a native absolute path as it is, a `file:` URL
 * converted; `null` for links (and for a Windows path on macOS, which names no file there).
 */
export function localPath(source: string): string | null {
  if (isAbsolute(source)) return source;
  if (/^file:/i.test(source))
    try {
      return fileURLToPath(source);
    } catch {
      return null; // a host macOS cannot reach as a path, or no absolute path
    }
  return null;
}

/** A source is a local file when it names a local path; anything else is a link. */
export const isLocal = (source: string) => localPath(source) !== null;

/** A Windows path where it names no file (macOS): shown by its file name, kept as a link. */
const foreignPath = (source: string) => !isLocal(source) && win32.isAbsolute(source);

const withoutFragment = (source: string) => {
  const hash = source.indexOf("#");
  return hash >= 0 ? source.slice(0, hash) : source;
};

/**
 * The identity shared by A12 and B25: a local file by its real path (the
 * path itself when that fails), a link by its text without the `#` fragment.
 */
export function keyOf(source: string): string {
  const path = localPath(source);
  if (path === null) return withoutFragment(source);
  try {
    return realpathSync.native(path);
  } catch {
    return path;
  }
}

/**
 * The key that 0.1.3 and earlier stored for `source`, when it differs from
 * `keyOf`; `null` otherwise. Then only a POSIX absolute path was a file, so a
 * local Windows path and a `file:` URL were keyed as link text (without the
 * fragment). Records saved under it stay readable and move to `keyOf` when
 * written again. On macOS it is `null` for every path and link.
 */
export function legacyKeyOf(source: string): string | null {
  return posix.isAbsolute(source) || !isLocal(source) ? null : withoutFragment(source);
}

/** The byte size of a local file; `null` for links and unreadable files. */
export function fileSize(source: string): number | null {
  const path = localPath(source);
  if (path === null) return null;
  try {
    return statSync(path).size;
  } catch {
    return null;
  }
}

/** Shorten to `max` characters by eliding the middle. */
export function middle(text: string, max: number): string {
  const chars = [...text];
  if (chars.length <= max) return text;
  if (max <= 1) return chars.slice(0, max).join("");
  const head = Math.ceil((max - 1) / 2);
  const tail = max - 1 - head;
  return `${chars.slice(0, head).join("")}…${tail ? chars.slice(-tail).join("") : ""}`;
}

const encoder = new TextEncoder();
/** UTF-8 length: the player counts menu texts in bytes. */
export const bytes = (text: string) => encoder.encode(text).length;

/** User-perceived characters; code points where the runtime has no `Intl.Segmenter`. */
const characters: (text: string) => string[] =
  typeof Intl.Segmenter === "function"
    ? ((graphemes) => (text: string) => [...graphemes.segment(text)].map(({ segment }) => segment))(
        new Intl.Segmenter(undefined, { granularity: "grapheme" }),
      )
    : (text: string) => [...text];

/**
 * Shorten to at most `max` UTF-8 bytes by eliding the middle (`…`, 3 bytes),
 * whole user-perceived characters only: a code point, surrogate pair, emoji
 * sequence or accented letter is kept or dropped as one.
 */
export function middleBytes(text: string, max: number): string {
  if (bytes(text) <= max) return text;
  const parts = characters(text).map((segment) => ({ segment, size: bytes(segment) }));
  const room = Math.max(0, max - bytes("…"));
  const take = (from: typeof parts, budget: number) => {
    const kept: string[] = [];
    for (const { segment, size } of from) {
      if (size > budget) break;
      kept.push(segment);
      budget -= size;
    }
    return kept;
  };
  const head = take(parts, Math.ceil(room / 2));
  const used = head.reduce((sum, segment) => sum + bytes(segment), 0);
  const tail = take(parts.slice(head.length).reverse(), room - used).reverse();
  return `${head.join("")}…${tail.join("")}`;
}

/**
 * Data made fit for a menu line: the player refuses control characters and
 * surrounding whitespace, so a control becomes a space and the edges are
 * trimmed (Unicode White_Space; NEL is a control).
 */
export const menuText = (text: string) =>
  text.replace(/\p{Cc}/gu, " ").replace(/^\s+|\s+$/gu, "");

const decode = (text: string) => {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
};

/**
 * What a menu shows for a source: a file's name (also of a Windows path on
 * macOS); for a link `host › last path segment` (decoded, at most 60
 * characters, never the `user:pass@` account, which stays in storage).
 */
export function displayName(source: string): string {
  const path = localPath(source);
  if (path !== null) return basename(path) || path;
  if (foreignPath(source)) return win32.basename(source) || source;
  let url: URL;
  try {
    url = new URL(source);
  } catch {
    return middle(source.replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/@]*@/i, "$1"), 60);
  }
  const host = url.hostname || url.protocol.replace(/:$/, "");
  // A URL path is `/`-separated on every OS.
  const segment = url.pathname
    .split("/")
    .filter((part) => part)
    .at(-1);
  return middle(segment ? `${host} › ${decode(segment)}` : host, 60);
}

/** `12:34`, or `1:02:03` from one hour on. */
export function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${two(m)}:${two(s)}` : `${two(m)}:${two(s)}`;
}

// The video and audio extensions of the feature requirements (appendix B.5).
const VIDEO = new Set(
  "mp4 m4v mkv webm mov avi wmv asf flv f4v ts m2ts mts mpg mpeg vob ogv 3gp rm rmvb".split(" "),
);
const AUDIO = new Set(
  "mp3 m4a aac flac wav ogg oga opus wma ape alac aif aiff dsf dff mka wv tta ac3 dts".split(" "),
);

/** `audio` / `video` by the media extension table; `null` when the extension says nothing. */
export function mediaKind(source: string): "audio" | "video" | null {
  let extension: string;
  const path = localPath(source);
  if (path !== null) extension = extname(path);
  else if (foreignPath(source)) extension = win32.extname(source);
  else
    try {
      extension = posix.extname(new URL(source).pathname);
    } catch {
      return null;
    }
  extension = extension.slice(1).toLowerCase();
  if (VIDEO.has(extension)) return "video";
  if (AUDIO.has(extension)) return "audio";
  return null;
}
