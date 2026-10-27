// Subtitle files next to a local video, loaded when it opens (mpv `sub-auto=exact`).
import { readdir } from "node:fs/promises";

/** How a folder is listed; tests replace `list` to slow it down. */
export const folders = { list: (folder: string): Promise<string[]> => readdir(folder) };

/** Extensions of the text subtitle files this plugin reads (formats.ts detects the format by content). */
export const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa", "vtt", "smi", "sami", "sub"];
/**
 * Video extensions, only to give a subtitle file to the video it names best:
 * `Movie.Extended.srt` belongs to `Movie.Extended.mkv`, not to `Movie.mkv`, when both are in the folder.
 */
const VIDEO_EXTENSIONS = [
  "mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "flv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "ogv", "3gp", "rm", "rmvb", "asf", "divx",
];
/**
 * At most this many tracks are loaded automatically per video: the player keeps 4 tracks per plugin and
 * source, and Open Subtitle…, Add Secondary Subtitle… and Reload Subtitles (new before old) need the rest.
 */
export const SIDECAR_LIMIT = 2;

const extension = (name: string) => (name.lastIndexOf(".") > 0 ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "");
/** File name without its last extension, compared case-insensitively and in NFC (macOS lists NFD names). */
const stem = (name: string) => (name.lastIndexOf(".") > 0 ? name.slice(0, name.lastIndexOf(".")) : name).normalize("NFC").toLowerCase();
/** `name`'s stem is `base`, or `base` + "." + a tag (`movie.en`, `movie.zh-Hans`, `movie.chs.forced`). */
const names = (base: string, name: string) => name === base || (name.startsWith(`${base}.`) && name.length > base.length + 1);

/**
 * The subtitle files among `entries` (the names in the video's folder) that belong to the video
 * `sourceName`: the video's name without its extension, optionally followed by "." and a tag,
 * then a subtitle extension; case-insensitive. The exact name comes first, then the others by name.
 * A file that names another video of the folder more closely (a longer name) belongs to that one.
 */
export function sidecars(sourceName: string, entries: readonly string[]): string[] {
  const base = stem(sourceName);
  if (!base) return [];
  const self = sourceName.normalize("NFC").toLowerCase();
  const others = entries
    .filter((name) => VIDEO_EXTENSIONS.includes(extension(name)) && name.normalize("NFC").toLowerCase() !== self)
    .map(stem)
    .filter((other) => other.length > base.length && names(base, other));
  return entries
    .filter((name) => SUBTITLE_EXTENSIONS.includes(extension(name)))
    .map((name) => ({ name, key: stem(name) }))
    .filter(({ key }) => names(base, key) && !others.some((other) => names(other, key)))
    .sort((a, b) => Number(b.key === base) - Number(a.key === base) || compare(a.name, b.name))
    .map(({ name }) => name);
}
const compare = (a: string, b: string) => {
  const x = a.normalize("NFC").toLowerCase(), y = b.normalize("NFC").toLowerCase();
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0;
};
