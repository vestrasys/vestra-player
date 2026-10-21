import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, unlinkSync } from "node:fs";
import { readdir, readFile, realpath, rename, stat, unlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { Tools } from "@vestra-player/sdk";
import { extensionOf, imageInfo, imageProblem } from "./images.ts";

/** Limits and delays; tests shorten them. */
export const limits = {
  /** J11: the cover cache keeps at most this many bytes… */
  cacheBytes: 200 * 1024 * 1024,
  /** …and drops entries not used for this long. */
  cacheAgeMs: 30 * 24 * 3600 * 1000,
  /** ffprobe and ffmpeg each get this long. */
  toolTimeoutMs: 10_000,
  /** Covers are scaled down to this side (decoded memory, not quality, is the limit). */
  coverSide: 1024,
};

/** What the audio scene shows for one track. */
export interface Track {
  title: string;
  artist: string;
  /** 专辑 · 年份, either part may be missing. */
  album: string;
  /** Absolute path of the cover in the cache, or null for the placeholder. */
  cover: string | null;
}

export interface Tags {
  title: string;
  artist: string;
  album: string;
  date: string;
}

interface Entry extends Tags {
  /** The cover's file name in the cache directory. */
  cover: string | null;
  bytes: number;
  lastUsed: number;
  /** A cover taken from the folder: re-read when that file changes. */
  folder?: { path: string; size: number; mtimeMs: number };
}

const EMPTY_TAGS: Tags = { title: "", artist: "", album: "", date: "" };

/** A URL (not a local path): no ffmpeg, the placeholder and the name only. */
export const isUrl = (source: string) => /^[a-z][a-z0-9+.-]*:\/\//i.test(source);

/** The file name of a path, or the last segment of a URL (its host when it has none). */
export function fileName(source: string) {
  if (isUrl(source)) {
    try {
      const url = new URL(source);
      const last = url.pathname.split("/").filter(Boolean).at(-1);
      return last ? decodeURIComponent(last) : url.hostname || source;
    } catch {
      return source;
    }
  }
  return source.split("/").filter(Boolean).at(-1) ?? source;
}

/** Text of a tag, trimmed and bounded (lyrics-sized values are not titles). */
const clean = (value: unknown) => (typeof value === "string" ? value.replace(/\s+/g, " ").trim().slice(0, 200) : "");

/** The arguments of the one ffprobe run per file: tags (format and stream) and attached pictures. */
export const probeArgs = (path: string) => [
  "-v",
  "error",
  "-show_entries",
  "format_tags:stream=index,codec_type:stream_tags:stream_disposition=attached_pic",
  "-of",
  "json",
  path,
];

/**
 * Tags and the attached picture's stream index from ffprobe's JSON. Tag keys
 * are matched without case (FLAC writes TITLE); container tags first, then
 * the first audio stream's (Ogg and Opus keep them there).
 */
export function parseProbe(output: string): { tags: Tags; attached: number | null } {
  const json = JSON.parse(output) as {
    format?: { tags?: Record<string, unknown> };
    streams?: { index?: number; codec_type?: string; disposition?: { attached_pic?: number }; tags?: Record<string, unknown> }[];
  };
  const lower = (tags: Record<string, unknown> | undefined) =>
    Object.fromEntries(Object.entries(tags ?? {}).map(([k, v]) => [k.toLowerCase(), v]));
  const streams = Array.isArray(json.streams) ? json.streams : [];
  const sources = [lower(json.format?.tags), lower(streams.find((s) => s.codec_type === "audio")?.tags)];
  const pick = (...keys: string[]) => {
    for (const tags of sources)
      for (const key of keys) {
        const value = clean(tags[key]);
        if (value) return value;
      }
    return "";
  };
  const picture = streams.find((s) => s.codec_type === "video" && s.disposition?.attached_pic === 1 && Number.isInteger(s.index));
  return {
    tags: { title: pick("title"), artist: pick("artist", "album_artist"), album: pick("album"), date: pick("date", "year") },
    attached: picture ? picture.index! : null,
  };
}

/** 专辑 · 年份 (the year is the first four digits of the date tag). */
export function albumLine(tags: Tags) {
  const year = tags.date.match(/\d{4}/)?.[0] ?? "";
  return [tags.album, year].filter(Boolean).join(" · ");
}

/** J11 folder covers in order: cover, folder, front; JPEG before PNG; any case. */
export function folderCover(names: string[]) {
  for (const stem of ["cover", "folder", "front"])
    for (const ext of ["jpg", "jpeg", "png"]) {
      const found = names.find((name) => name.toLowerCase() === `${stem}.${ext}`);
      if (found) return found;
    }
  return null;
}

/** ffmpeg: one frame of `map` from `input`, scaled down to `side`, as JPEG on stdout. */
export const extractArgs = (input: string, map: string, side: number) => [
  "-v",
  "error",
  "-nostdin",
  "-i",
  input,
  "-map",
  map,
  "-frames:v",
  "1",
  "-an",
  "-vf",
  `scale='min(${side},iw)':'min(${side},ih)':force_original_aspect_ratio=decrease`,
  "-f",
  "image2pipe",
  "-c:v",
  "mjpeg",
  "-q:v",
  "3",
  "-",
];

const isAbort = (error: unknown) => (error as { code?: string; name?: string })?.code === "aborted" || (error as Error)?.name === "AbortError";

/**
 * J11 covers and tags, cached in `<dataDirectory>/cache/covers/`: `<key>.jpg`
 * (or `.png`) per track, with `tags.json` beside them. The key is
 * sha1(realpath + size + mtime); the cache keeps 200 MB and 30 days, dropping
 * the least recently used first. Only answers ffprobe gave are cached, so a
 * missing tool or a timeout is tried again next time.
 */
export class CoverCache {
  readonly directory: string;
  readonly #tools: Tools;
  #entries = new Map<string, Entry>();
  #saving: Promise<void> = Promise.resolve();
  #ready = false;

  constructor(dataDirectory: string, tools: Tools) {
    this.directory = join(dataDirectory, "cache", "covers");
    this.#tools = tools;
  }

  /** Read `tags.json`, drop files no entry names (left by an interrupted run), apply the limits. */
  load() {
    if (this.#ready) return;
    this.#ready = true;
    try {
      mkdirSync(this.directory, { recursive: true });
      const raw = JSON.parse(readFileSync(join(this.directory, "tags.json"), "utf8")) as { entries?: Record<string, Entry> };
      for (const [key, entry] of Object.entries(raw.entries ?? {}))
        if (/^[0-9a-f]{40}$/.test(key) && entry && typeof entry === "object" && typeof entry.lastUsed === "number")
          this.#entries.set(key, {
            ...EMPTY_TAGS,
            ...entry,
            title: clean(entry.title),
            artist: clean(entry.artist),
            album: clean(entry.album),
            date: clean(entry.date),
            cover: typeof entry.cover === "string" && /^[0-9a-f]{40}\.(jpg|png)$/.test(entry.cover) ? entry.cover : null,
            bytes: Number(entry.bytes) || 0,
          });
    } catch {
      // Missing or unreadable: start empty.
    }
    try {
      const named = new Set([...this.#entries.values()].map((e) => e.cover));
      for (const name of readdirSync(this.directory))
        if (name !== "tags.json" && !named.has(name)) unlinkSync(join(this.directory, name));
    } catch {
      // Best effort; the next start tries again.
    }
    if (this.#evict(null)) this.#save();
  }

  /** Entries and bytes (the settings page shows the size). */
  get usage() {
    let bytes = 0;
    for (const entry of this.#entries.values()) bytes += entry.bytes;
    return { entries: this.#entries.size, bytes };
  }

  /** Remove every cached cover and tag. */
  async clear() {
    this.load();
    const names = [...this.#entries.values()].map((e) => e.cover).filter((n): n is string => !!n);
    this.#entries.clear();
    await Promise.all(names.map((name) => unlink(join(this.directory, name)).catch(() => {})));
    this.#save();
    await this.#saving;
  }

  /** Writes in flight (tests wait for them). */
  flush() {
    return this.#saving;
  }

  /**
   * What to show for `source`. Never rejects except with an abort: a file that
   * cannot be read, a missing tool or a bad picture all end in the placeholder.
   */
  async lookup(source: string, signal: AbortSignal): Promise<Track> {
    this.load();
    const name = fileName(source);
    const bare: Track = { title: name, artist: "", album: "", cover: null };
    if (isUrl(source)) return bare;
    let real: string;
    let key: string;
    try {
      real = await realpath(source);
      const info = await stat(real);
      key = createHash("sha1").update(`${real}\n${info.size}\n${info.mtimeMs}`).digest("hex");
    } catch {
      return bare;
    }
    signal.throwIfAborted();
    const cached = await this.#valid(key);
    signal.throwIfAborted();
    if (cached) {
      cached.lastUsed = Date.now();
      this.#save();
      return this.#track(cached, name);
    }

    let tags = EMPTY_TAGS;
    let attached: number | null = null;
    let probed = false;
    try {
      const { stdout } = await this.#tools.run("ffprobe", probeArgs(real), {
        timeoutMs: limits.toolTimeoutMs,
        maxOutput: 4 * 1024 * 1024,
        signal,
      });
      ({ tags, attached } = parseProbe(Buffer.from(stdout).toString("utf8")));
      probed = true;
    } catch (error) {
      if (isAbort(error) || signal.aborted) throw error;
    }

    let cover: { name: string; bytes: number } | null = null;
    let folder: Entry["folder"];
    if (attached !== null) cover = await this.#extract(real, `0:${attached}`, key, signal);
    if (!cover) {
      try {
        const names = await readdir(dirname(real));
        const found = folderCover(names);
        if (found) {
          const path = join(dirname(real), found);
          const info = await stat(path);
          cover = await this.#folderCover(path, key, signal);
          if (cover) folder = { path, size: info.size, mtimeMs: info.mtimeMs };
        }
      } catch (error) {
        if (isAbort(error) || signal.aborted) throw error;
      }
    }
    signal.throwIfAborted();
    const entry: Entry = { ...tags, cover: cover?.name ?? null, bytes: cover?.bytes ?? 0, lastUsed: Date.now(), ...(folder ? { folder } : {}) };
    if (probed) {
      this.#entries.set(key, entry);
      this.#evict(key);
      this.#save();
    }
    return this.#track(entry, name);
  }

  #track(entry: Entry, name: string): Track {
    return {
      title: entry.title || name,
      artist: entry.artist,
      album: albumLine(entry),
      cover: entry.cover ? join(this.directory, entry.cover) : null,
    };
  }

  /** The cached entry while its cover file is still there and its folder cover unchanged. */
  async #valid(key: string) {
    const entry = this.#entries.get(key);
    if (!entry) return null;
    try {
      if (entry.cover) await stat(join(this.directory, entry.cover));
      if (entry.folder) {
        const info = await stat(entry.folder.path);
        if (info.size !== entry.folder.size || info.mtimeMs !== entry.folder.mtimeMs) throw new Error("changed");
      }
      return entry;
    } catch {
      this.#entries.delete(key);
      return null;
    }
  }

  /** Run ffmpeg and store the picture as `<key>.jpg`; null when it fails or is not a usable image. */
  async #extract(input: string, map: string, key: string, signal: AbortSignal) {
    try {
      const { stdout } = await this.#tools.run("ffmpeg", extractArgs(input, map, limits.coverSide), {
        timeoutMs: limits.toolTimeoutMs,
        signal,
      });
      return await this.#store(stdout, key);
    } catch (error) {
      if (isAbort(error) || signal.aborted) throw error;
      return null;
    }
  }

  /** A folder picture: scaled through ffmpeg when there is one, else copied as it is if the player can show it. */
  async #folderCover(path: string, key: string, signal: AbortSignal) {
    const scaled = await this.#extract(path, "0:v:0", key, signal);
    if (scaled) return scaled;
    try {
      if ((await stat(path)).size > 16 * 1024 * 1024) return null;
      return await this.#store(await readFile(path), key);
    } catch {
      return null;
    }
  }

  async #store(bytes: Uint8Array, key: string) {
    const info = imageInfo(bytes);
    if (!info || imageProblem(bytes)) return null;
    const name = `${key}.${extensionOf(info)}`;
    const path = join(this.directory, name);
    const temporary = `${path}.${process.pid}.tmp`;
    await writeFile(temporary, bytes);
    await rename(temporary, path);
    return { name, bytes: bytes.length };
  }

  /** Drop entries older than 30 days, then the least recently used beyond 200 MB (never `keep`). */
  #evict(keep: string | null) {
    const now = Date.now();
    const byAge = [...this.#entries].sort((a, b) => a[1].lastUsed - b[1].lastUsed);
    let total = byAge.reduce((sum, [, e]) => sum + e.bytes, 0);
    let changed = false;
    for (const [key, entry] of byAge) {
      if (key === keep) continue;
      if (now - entry.lastUsed <= limits.cacheAgeMs && total <= limits.cacheBytes) continue;
      this.#entries.delete(key);
      total -= entry.bytes;
      changed = true;
      if (entry.cover) void unlink(join(this.directory, entry.cover)).catch(() => {});
    }
    return changed;
  }

  /** `tags.json`, written whole (temporary file and rename), one write at a time. */
  #save() {
    const body = JSON.stringify({ version: 1, entries: Object.fromEntries(this.#entries) });
    const path = join(this.directory, "tags.json");
    this.#saving = this.#saving.then(async () => {
      try {
        const temporary = `${path}.${process.pid}.tmp`;
        await writeFile(temporary, body);
        await rename(temporary, path);
      } catch {
        // The cache is only a cache.
      }
    });
  }
}
