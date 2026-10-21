import { createHash } from "node:crypto";
import { mkdir, realpath, rm, stat, unlink } from "node:fs/promises";
import { rmSync } from "node:fs";
import { join } from "node:path";
import type { CommandState, ImageRef, PlayerState, Plugin } from "../../sdk/plugin/index.ts";
import type { SettingsStore } from "./settings.ts";

export const NEEDS_FFMPEG = "需要 ffmpeg";
/** Cached pictures per source; the least recently shown go first. */
export const MAX_PER_SOURCE = 200;
/** The pointer rests this long on an uncached position before ffmpeg runs. */
export const SETTLE_MS = 80;
export const TIMEOUT_MS = 3000;
/** The settle timer's clock; tests replace it to fire the timer themselves. */
export const clock = {
  setTimeout: (run: () => void, ms: number): unknown => setTimeout(run, ms),
  clearTimeout: (timer: unknown) => clearTimeout(timer as ReturnType<typeof setTimeout>),
};
/** Failures in a row after which a source is not tried again. */
export const MAX_FAILURES = 5;

/**
 * The cache slot of a hover time: the duration in steps of 0.5 % (at least 2
 * seconds). `seconds` is where ffmpeg seeks: the slot's start, kept inside
 * the source.
 */
export function slotOf(seconds: number, duration: number): { index: number; seconds: number } {
  const step = Math.max(duration * 0.005, 2);
  const index = Math.max(0, Math.floor(Math.min(seconds, duration) / step));
  return { index, seconds: slotSeconds(index, duration) };
}

/** Where ffmpeg seeks for a slot: its start, kept half a second inside the end. */
export const slotSeconds = (index: number, duration: number) =>
  Math.max(0, Math.min(index * Math.max(duration * 0.005, 2), duration - 0.5));

/** `-ss` before `-i`: a fast seek to the keyframe; no shell, so the path is passed as is. */
export const ffmpegArgs = (source: string, seconds: number, out: string) => [
  "-nostdin",
  "-v",
  "error",
  "-ss",
  seconds.toFixed(3),
  "-i",
  source,
  "-frames:v",
  "1",
  "-vf",
  "scale=160:90:force_original_aspect_ratio=decrease",
  "-q:v",
  "5",
  "-y",
  out,
];

type Source = {
  sourceId: number;
  path: string;
  duration: number;
  /** `thumbs/<hash of realpath + mtime>`, found on the first run. */
  dir: Promise<string> | null;
  /** Slot index → picture, in order of use (oldest first). */
  cache: Map<number, { file: string; image: ImageRef }>;
  failures: number;
  /** Ends this source's ffmpeg when another source opens. */
  abort: AbortController;
};

/**
 * I07 进度条悬停显示缩略图: the bar reports the hovered fraction (at most every
 * 50 ms); a cached slot shows at once, an uncached one is made by ffmpeg once
 * the pointer rests 80 ms. One ffmpeg at a time: a target that changed while
 * it ran only gets cached, and the newest target runs next. Pictures live in
 * `dataDirectory/thumbs/<source>/`, emptied at start and when the source changes.
 */
export class Thumbnails {
  /** ffmpeg was found (`null` until the first answer). */
  available: boolean | null = null;
  /** The picture the bar shows; `null`: a black box until one is ready. */
  current: ImageRef | null = null;
  #root: string | null = null;
  #seen: number | null = null;
  #source: Source | null = null;
  #target: number | null = null;
  #timer: unknown = null;
  #running = false;
  readonly #scope: Plugin;
  readonly #settings: SettingsStore;
  readonly #video: (sourceId: number) => boolean | null;

  /** `video`: whether the source has video (`stats().video`), `null` while unknown. */
  constructor(scope: Plugin, settings: SettingsStore, video: (sourceId: number) => boolean | null) {
    this.#scope = scope;
    this.#settings = settings;
    this.#video = video;
    try {
      this.#root = join(scope.dataDirectory, "thumbs");
      rmSync(this.#root, { recursive: true, force: true });
    } catch {
      this.#root = null; // no data directory: nowhere to keep pictures
    }
    scope.onDispose(() => this.#clearTimer());
  }

  /** Look for ffmpeg (at start, and on each new source while it is missing). */
  async probe() {
    const found = this.#root !== null && (await this.#scope.tools.available("ffmpeg").catch(() => false));
    this.available = found;
    this.#scope.update();
  }

  command(): CommandState {
    const missing = this.available === false;
    return {
      enabled: !missing,
      checked: this.#settings.value.hoverThumbnail,
      busy: false,
      reason: missing ? NEEDS_FFMPEG : null,
    };
  }

  /** The bar draws the picture box: the option is on and this source can have pictures. */
  active(state: PlayerState = this.#scope.player.state): boolean {
    const source = this.#source;
    return (
      this.#settings.value.hoverThumbnail &&
      this.available === true &&
      source !== null &&
      source.sourceId === state.sourceId &&
      source.failures < MAX_FAILURES &&
      state.phase === "ready" &&
      this.#video(state.sourceId) === true
    );
  }

  /** Call on every state change: a new source drops the old one's pictures. */
  observe(state: PlayerState) {
    if (this.#source?.sourceId === state.sourceId) return;
    if (this.#seen !== state.sourceId) {
      this.#seen = state.sourceId;
      if (this.#source) this.#forget(this.#source);
      this.#source = null;
      this.#target = null;
      this.current = null;
      this.#clearTimer();
      if (this.available === false && this.#root !== null) void this.probe(); // ffmpeg may be installed by now
    }
    // A local file with a known length (it may be known only once ready).
    const { kind, source, duration } = state;
    if (kind === "media" && source !== null && source.startsWith("/") && duration !== null && duration > 0)
      this.#source = this.#fresh(state.sourceId, source, duration);
  }

  /** The pointer is over `fraction` of the seek line. */
  hover(fraction: number) {
    const state = this.#scope.player.state;
    const source = this.#source;
    if (!this.active(state) || !source || !Number.isFinite(fraction)) return;
    const { index } = slotOf(Math.min(Math.max(fraction, 0), 1) * source.duration, source.duration);
    const hit = source.cache.get(index);
    if (hit) {
      source.cache.delete(index);
      source.cache.set(index, hit); // most recently used
      this.current = hit.image;
      this.#target = null;
      this.#clearTimer();
      this.#scope.update();
      return;
    }
    if (this.#target === index) return;
    this.#target = index; // the previous picture stays until this one is ready
    this.#clearTimer();
    this.#timer = clock.setTimeout(() => {
      this.#timer = null;
      void this.#pump();
    }, SETTLE_MS);
  }

  /** The pointer left the seek line: nothing more to make. */
  hoverEnded() {
    this.#target = null;
    this.#clearTimer();
  }

  #fresh(sourceId: number, path: string, duration: number): Source {
    return { sourceId, path, duration, dir: null, cache: new Map(), failures: 0, abort: new AbortController() };
  }

  #forget(source: Source) {
    source.abort.abort();
    const dir = source.dir;
    source.dir = null;
    source.cache.clear();
    if (dir) void dir.then((path) => rm(path, { recursive: true, force: true })).catch(() => {});
  }

  #clearTimer() {
    if (this.#timer !== null) clock.clearTimeout(this.#timer);
    this.#timer = null;
  }

  #dirOf(source: Source): Promise<string> {
    source.dir ??= (async () => {
      const real = await realpath(source.path);
      const { mtimeMs } = await stat(real);
      const hash = createHash("sha1").update(`${real}\n${mtimeMs}`).digest("hex").slice(0, 16);
      const dir = join(this.#root!, hash);
      await mkdir(dir, { recursive: true });
      return dir;
    })();
    return source.dir;
  }

  async #pump() {
    const source = this.#source;
    const index = this.#target;
    if (this.#running || !source || index === null || !this.active()) return;
    const hit = source.cache.get(index);
    if (hit) {
      this.current = hit.image;
      this.#target = null;
      this.#scope.update();
      return;
    }
    this.#running = true;
    let made: { file: string; image: ImageRef } | null = null;
    try {
      const dir = await this.#dirOf(source);
      const file = join(dir, `${index}.jpg`);
      const seconds = slotSeconds(index, source.duration);
      await this.#scope.tools.run("ffmpeg", ffmpegArgs(source.path, seconds, file), {
        timeoutMs: TIMEOUT_MS,
        maxOutput: 64 * 1024,
        signal: source.abort.signal,
      });
      if ((await stat(file)).size === 0) throw new Error("empty picture");
      made = { file, image: this.#scope.ui.image(file) };
    } catch (error) {
      if ((error as { code?: string })?.code === "unavailable") {
        this.available = false; // ffmpeg went away: the menu item turns grey
        this.#scope.update();
      }
      // A failed picture shows the time only, without a message.
      if (this.#source === source && !source.abort.signal.aborted) source.failures++;
    } finally {
      this.#running = false;
    }
    if (this.#source !== source) {
      if (made) void unlink(made.file).catch(() => {});
      return;
    }
    if (made) {
      source.failures = 0;
      this.#store(source, index, made);
      if (this.#target === index) {
        this.current = made.image;
        this.#target = null;
      }
      this.#scope.update();
    } else {
      if (this.#target === index) this.#target = null; // not tried again until the pointer comes back
      if (source.failures >= MAX_FAILURES) this.#scope.update();
    }
    // The pointer moved on meanwhile: make the newest target, unless it is still settling.
    if (this.#target !== null && this.#timer === null) void this.#pump();
  }

  #store(source: Source, index: number, entry: { file: string; image: ImageRef }) {
    source.cache.set(index, entry);
    for (const [key, old] of source.cache) {
      if (source.cache.size <= MAX_PER_SOURCE) break;
      if (old.image === this.current) continue; // never remove what the bar shows
      source.cache.delete(key);
      void unlink(old.file).catch(() => {});
    }
  }
}
