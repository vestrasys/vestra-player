import { createHash } from "node:crypto";
import { mkdir, readdir, realpath, rename, stat, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { ImageRef, PluginError, Tools } from "@vestra-player/sdk";
import { mediaKind } from "./media.ts";
import { lengthOf, type Queue, type Row } from "./queue.ts";

/** Runs ffmpeg with `args` and resolves its stdout; rejects on failure, timeout or abort. */
export type ThumbRunner = (args: string[], options: { signal: AbortSignal; timeoutMs: number }) => Promise<Uint8Array>;

/** The picture's box: twice the 96×54 the list draws (sharp on high-density screens). */
export const THUMB_WIDTH = 192;
export const THUMB_HEIGHT = 108;
/** ffmpeg processes at most (one thread each). */
export const THUMB_LIMIT = 2;
/** Cache lookups (realpath + stat) at most at a time. */
export const LOOKUP_LIMIT = 4;
export const THUMB_TIMEOUT_MS = 10_000;
/** A 192×108 JPEG is 3–10 KB; more than this is not one. */
export const THUMB_OUTPUT_LIMIT = 512 * 1024;
/** Pictures kept in `<dataDirectory>/thumbs` (least recently used dropped first; about 7 KB each). */
export const THUMB_CACHE_LIMIT = 5000;
/** Where in a video the picture is taken (a fraction of its length): past the opening titles. */
export const THUMB_AT = 0.1;
/** After a source opens, generation waits this long (the open and its first seconds come first). */
export const HOLD_AFTER_OPEN_MS = 3000;
/** Pictures arriving within this long reach the list as one update. */
export const THUMB_NOTIFY_MS = 100;

const scaled = `scale=${THUMB_WIDTH}:${THUMB_HEIGHT}:force_original_aspect_ratio=decrease:flags=bilinear,format=yuvj420p`;

/**
 * One small JPEG of `path` on stdout.
 *
 * - Video (`at` in seconds): the keyframe at or before `at` (`-skip_frame nokey`,
 *   `-ss` before `-i` with `-noaccurate_seek`: nothing after the keyframe is decoded),
 *   from the first video stream that is not cover art (`0:V:0`).
 * - Audio (`at` null): the embedded cover art, the first video stream (`0:v:0`), about
 *   0.15 s; a file without one fails ("matches no streams"), cached as "no picture".
 * - One thread for the decoder, the filters and the encoder: the player keeps the other cores.
 */
export function thumbArgs(path: string, at: number | null): string[] {
  // Cover art is one picture; `-skip_frame` on a file without a video decoder fails at once.
  const keyframes = at === null ? [] : ["-skip_frame", "nokey"];
  const seek = at !== null && at > 0 ? ["-noaccurate_seek", "-ss", at.toFixed(3)] : [];
  return [
    "-nostdin",
    "-v",
    "error",
    "-threads",
    "1",
    ...keyframes,
    ...seek,
    "-i",
    path,
    "-map",
    at === null ? "0:v:0" : "0:V:0",
    "-an",
    "-sn",
    "-dn",
    "-frames:v",
    "1",
    "-filter_threads",
    "1",
    "-vf",
    scaled,
    "-threads",
    "1",
    "-q:v",
    "5",
    "-f",
    "image2pipe",
    "-c:v",
    "mjpeg",
    "-",
  ];
}

/** ffmpeg through `plugin.tools` (lookup, no shell, timeout, abort and output bound). */
export const ffmpegRunner =
  (tools: Tools): ThumbRunner =>
  async (args, { signal, timeoutMs }) =>
    (await tools.run("ffmpeg", args, { signal, timeoutMs, maxOutput: THUMB_OUTPUT_LIMIT })).stdout;

/** What a row shows: a picture, or none (the list draws its placeholder). */
type Job = { rowId: string; hash: string; path: string; at: number | null };

/**
 * Thumbnails of local rows (0.1.11), made lazily like the durations (`probe.ts`):
 *
 * - Only the rows `want` names (those on screen plus a few) are looked at; a row
 *   that leaves them stops waiting, and its running ffmpeg is ended.
 * - A row looks up the disk cache first (realpath + size + mtime → a JPEG in
 *   `<dataDirectory>/thumbs`); only a miss runs ffmpeg, at most `THUMB_LIMIT` at a time.
 * - `hold` pauses the ffmpeg runs (while a source opens and for its first seconds);
 *   running ones are ended and done again later. Cache hits go on.
 * - A file ffmpeg cannot read is cached as "no picture" and not tried again; a
 *   timeout is not tried again this session; without ffmpeg nothing is run.
 * - Links and files that are not audio or video get no picture.
 */
export class Thumbnailer {
  /** hash -> has a picture (false: ffmpeg found none); insertion order = LRU order. */
  private index = new Map<string, boolean>();
  /** The cache changed since the last `export()`. */
  dirty = false;
  /** rowId -> its picture, or null for none. */
  private results = new Map<string, ImageRef | null>();
  /** One reference per picture, so an unchanged list is not sent again. */
  private refs = new Map<string, ImageRef>();
  /** Timed out this session (not cached: may work later). */
  private skipped = new Set<string>();
  private wanted: string[] = [];
  private wantedKey = "";
  private looking = new Map<string, AbortController>();
  private waiting = new Map<string, Job>();
  private running = new Map<string, AbortController>();
  private heldUntil = 0;
  private holdTimer: ReturnType<typeof setTimeout> | null = null;
  private notify: ReturnType<typeof setTimeout> | null = null;
  private notifiedAt = Number.NEGATIVE_INFINITY;
  /** ffmpeg is missing: no more runs. */
  private noTool = false;
  /** The folder was swept once (files the index does not know are removed). */
  private swept: Promise<void> | null = null;

  private queue: Queue;
  private runner: ThumbRunner;
  private dir: string | null;
  private image: (path: string) => ImageRef;
  private changed: () => void;
  constructor(
    queue: Queue,
    runner: ThumbRunner,
    dir: string | null,
    image: (path: string) => ImageRef,
    changed: () => void,
  ) {
    this.queue = queue;
    this.runner = runner;
    this.dir = dir;
    this.image = image;
    this.changed = changed;
  }

  /** The row's picture: a reference, null for none, undefined while unknown. */
  picture(rowId: string): ImageRef | null | undefined {
    return this.results.get(rowId);
  }

  /**
   * Look only at these rows, in this order (the rows on screen first). Rows that left
   * stop: a lookup or ffmpeg run of theirs is ended. Called on every list update;
   * unchanged rows only pick up rows whose length has just been read.
   */
  want(rowIds: readonly string[]) {
    const key = rowIds.join("\n");
    if (key !== this.wantedKey) {
      this.wantedKey = key;
      this.wanted = [...rowIds];
      const keep = new Set(rowIds);
      for (const id of [...this.waiting.keys()]) if (!keep.has(id)) this.waiting.delete(id);
      for (const map of [this.looking, this.running])
        for (const [id, controller] of [...map])
          if (!keep.has(id)) {
            controller.abort();
            map.delete(id);
          }
    }
    this.pump();
  }

  /** Pause ffmpeg runs until `until` (ms, `Date.now()` clock; Infinity: until called again). */
  hold(until: number) {
    this.heldUntil = until;
    if (this.holdTimer) clearTimeout(this.holdTimer);
    this.holdTimer = null;
    if (this.held()) {
      // Ended now and made again later (a keyframe takes a fraction of a second).
      for (const [id, controller] of [...this.running]) {
        controller.abort();
        this.running.delete(id);
      }
      if (Number.isFinite(until))
        this.holdTimer = setTimeout(() => {
          this.holdTimer = null;
          this.pump();
        }, until - Date.now());
    } else this.pump();
  }
  private held() {
    return Date.now() < this.heldUntil;
  }

  /** The row left the list. */
  cancel(rowId: string) {
    this.results.delete(rowId);
    this.waiting.delete(rowId);
    for (const map of [this.looking, this.running]) {
      map.get(rowId)?.abort();
      map.delete(rowId);
    }
  }
  cancelAll() {
    this.want([]);
    if (this.notify) clearTimeout(this.notify);
    if (this.holdTimer) clearTimeout(this.holdTimer);
    this.notify = this.holdTimer = null;
  }

  /** Processes running (for tests). */
  get active() {
    return this.running.size;
  }

  /** The cache index as saved in `thumb-cache.json`. */
  export(): [string, boolean][] {
    this.dirty = false;
    return [...this.index];
  }
  import(entries: unknown) {
    if (!Array.isArray(entries)) return;
    for (const entry of entries.slice(-THUMB_CACHE_LIMIT))
      if (Array.isArray(entry) && typeof entry[0] === "string" && /^[0-9a-f]{40}$/.test(entry[0]) && typeof entry[1] === "boolean")
        this.index.set(entry[0], entry[1]);
  }

  private file(hash: string) {
    return join(this.dir!, `${hash}.jpg`);
  }
  private ref(hash: string) {
    let ref = this.refs.get(hash);
    if (!ref) this.refs.set(hash, (ref = this.image(this.file(hash))));
    return ref;
  }
  private remember(hash: string, picture: boolean) {
    this.index.delete(hash);
    this.index.set(hash, picture);
    while (this.index.size > THUMB_CACHE_LIMIT) {
      const [oldest, had] = this.index.entries().next().value!;
      this.index.delete(oldest);
      this.refs.delete(oldest);
      if (had) void unlink(this.file(oldest)).catch(() => {});
    }
    this.dirty = true;
  }

  /** Remove pictures the index does not list (a crash between writing one and saving the index). */
  private sweep() {
    this.swept ??= (async () => {
      try {
        await mkdir(this.dir!, { recursive: true });
        const names = new Set(await readdir(this.dir!));
        for (const [hash, picture] of [...this.index])
          if (picture && !names.has(`${hash}.jpg`)) {
            this.index.delete(hash);
            this.dirty = true;
          }
        for (const name of names) {
          const hash = name.replace(/\.jpg$/, "");
          if (!this.index.get(hash)) await unlink(join(this.dir!, name)).catch(() => {});
        }
      } catch {
        /* an unreadable folder: writes fail later and the rows get no picture */
      }
    })();
    return this.swept;
  }

  private pump() {
    if (!this.dir) return;
    for (const rowId of this.wanted) {
      if (this.looking.size >= LOOKUP_LIMIT) break;
      if (this.results.has(rowId) || this.looking.has(rowId) || this.waiting.has(rowId) || this.running.has(rowId))
        continue;
      const row = this.queue.row(rowId);
      if (!row) continue;
      const kind = row.kind === "file" ? mediaKind(row.source) : null;
      if (!kind) {
        this.results.set(rowId, null);
        continue;
      }
      // The picture is taken at a tenth of the length: wait for ffprobe's answer.
      if (kind === "video" && lengthOf(row) === null && row.probe === "pending") continue;
      const controller = new AbortController();
      this.looking.set(rowId, controller);
      void this.lookup(row, kind, controller.signal).finally(() => {
        if (this.looking.get(rowId) === controller) this.looking.delete(rowId);
        this.pump();
      });
    }
    if (this.held() || this.noTool) return;
    for (const rowId of this.wanted) {
      if (this.running.size >= THUMB_LIMIT) break;
      const job = this.waiting.get(rowId);
      if (!job) continue;
      this.waiting.delete(rowId);
      const controller = new AbortController();
      this.running.set(rowId, controller);
      void this.make(job, controller.signal).finally(() => {
        if (this.running.get(rowId) === controller) this.running.delete(rowId);
        this.pump();
      });
    }
  }

  /** Cache hit: the picture (or none) at once; miss: wait for an ffmpeg slot. */
  private async lookup(row: Row, kind: "video" | "audio", signal: AbortSignal) {
    let hash: string, real: string;
    try {
      await this.sweep();
      real = await realpath(row.source);
      const info = await stat(real);
      if (!info.isFile()) throw new Error("not a file");
      hash = createHash("sha1").update(`${real}\n${info.size}\n${info.mtimeMs}`).digest("hex");
    } catch {
      if (!signal.aborted) this.results.set(row.rowId, null);
      return;
    }
    if (signal.aborted) return;
    const known = this.index.get(hash);
    if (known !== undefined) {
      this.remember(hash, known);
      return this.answer(row.rowId, known ? this.ref(hash) : null);
    }
    if (this.noTool || this.skipped.has(hash)) return void this.results.set(row.rowId, null);
    const length = lengthOf(row);
    const at = kind === "audio" ? null : length !== null ? length * THUMB_AT : 0;
    this.waiting.set(row.rowId, { rowId: row.rowId, hash, path: real, at });
  }

  private async make(job: Job, signal: AbortSignal) {
    /** Ended by `hold` (not scrolled away): wait for a slot again. */
    const ended = () => {
      if (this.wanted.includes(job.rowId) && !this.results.has(job.rowId)) this.waiting.set(job.rowId, job);
    };
    const run = (at: number | null) => this.runner(thumbArgs(job.path, at), { signal, timeoutMs: THUMB_TIMEOUT_MS });
    let jpeg: Uint8Array;
    try {
      jpeg = await run(job.at);
      // Past the last keyframe (a length that was wrong): the first one instead.
      if (!jpeg.length && job.at && !signal.aborted) jpeg = await run(0);
    } catch (error) {
      if (signal.aborted) return ended();
      const code = (error as PluginError).code;
      if (code === "unavailable") this.noTool = true;
      // ffmpeg read the file and found no picture: cached, never tried again.
      if (code === "failed") this.remember(job.hash, false);
      else this.skipped.add(job.hash);
      return this.answer(job.rowId, null);
    }
    if (signal.aborted) return ended();
    const picture = jpeg.length > 0;
    if (picture)
      try {
        const file = this.file(job.hash);
        const temp = `${file}.${process.pid}.tmp`;
        await writeFile(temp, jpeg);
        await rename(temp, file);
      } catch {
        this.skipped.add(job.hash);
        return this.answer(job.rowId, null);
      }
    this.remember(job.hash, picture);
    this.answer(job.rowId, picture ? this.ref(job.hash) : null);
  }

  /**
   * A row's picture is known. Only a picture changes what the list shows; the list
   * hears of it at once after a quiet spell, else once per `THUMB_NOTIFY_MS`.
   */
  private answer(rowId: string, picture: ImageRef | null) {
    if (!this.queue.row(rowId)) return;
    this.results.set(rowId, picture);
    if (!picture || this.notify) return;
    const now = performance.now();
    if (now - this.notifiedAt >= THUMB_NOTIFY_MS) {
      this.notifiedAt = now;
      return this.changed();
    }
    this.notify = setTimeout(() => {
      this.notify = null;
      this.notifiedAt = performance.now();
      this.changed();
    }, THUMB_NOTIFY_MS - (now - this.notifiedAt));
  }
}
