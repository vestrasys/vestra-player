import { realpathSync, statSync } from "node:fs";
import { basename, extname } from "node:path";

/** A source is a local file when it is an absolute path; anything else is a link. */
export const isLocal = (source: string) => source.startsWith("/");

/**
 * The identity shared by A12 and B25: a local file by its real path (the
 * original path when that fails), a link by its text without the `#` fragment.
 */
export function keyOf(source: string): string {
  if (!isLocal(source)) {
    const hash = source.indexOf("#");
    return hash >= 0 ? source.slice(0, hash) : source;
  }
  try {
    return realpathSync.native(source);
  } catch {
    return source;
  }
}

/** The byte size of a local file; `null` for links and unreadable files. */
export function fileSize(source: string): number | null {
  if (!isLocal(source)) return null;
  try {
    return statSync(source).size;
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

const decode = (text: string) => {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
};

/**
 * What a menu shows for a source: a file's name; for a link
 * `host › last path segment` (decoded, at most 60 characters, never the
 * `user:pass@` account, which stays in storage).
 */
export function displayName(source: string): string {
  if (isLocal(source)) return basename(source) || source;
  let url: URL;
  try {
    url = new URL(source);
  } catch {
    return middle(source.replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/@]*@/i, "$1"), 60);
  }
  const host = url.hostname || url.protocol.replace(/:$/, "");
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

// doc/product/feature-requirements.md appendix B.5.
const VIDEO = new Set(
  "mp4 m4v mkv webm mov avi wmv asf flv f4v ts m2ts mts mpg mpeg vob ogv 3gp rm rmvb".split(" "),
);
const AUDIO = new Set(
  "mp3 m4a aac flac wav ogg oga opus wma ape alac aif aiff dsf dff mka wv tta ac3 dts".split(" "),
);

/** `audio` / `video` by the media extension table; `null` when the extension says nothing. */
export function mediaKind(source: string): "audio" | "video" | null {
  let path = source;
  if (!isLocal(source)) {
    try {
      path = new URL(source).pathname;
    } catch {
      return null;
    }
  }
  const extension = extname(path).slice(1).toLowerCase();
  if (VIDEO.has(extension)) return "video";
  if (AUDIO.has(extension)) return "audio";
  return null;
}
