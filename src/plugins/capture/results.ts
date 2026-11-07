import { mkdir, readdir, stat, writeFile } from "node:fs/promises";
import { basename, extname, join } from "node:path";
import type { Scope } from "@vestra-player/sdk";
import type { Method } from "./clip.ts";
import type { Kind } from "./model.ts";

/**
 * Finished captures shown in the tab's Results (proposal §3.2 ④). The most
 * recent twenty are kept in `<dataDirectory>/results.json` across restarts
 * (open question 2: finished results only; tasks and failed input stay in the
 * session). A row whose file is gone says so.
 */
export interface Saved {
  id: string;
  kind: Kind;
  /** The file (frame, clip) or folder (burst). */
  path: string;
  /** A burst's first frame (its thumbnail and Copy). */
  first: string;
  /** A clip's method; "" for frames and bursts. */
  method: Method | "";
  bytes: number;
  /** A burst's frames (1 otherwise). */
  count: number;
  /** Media times: the frame, or the clip's range as written. */
  start: number;
  end: number;
  /** When it finished (ms since the epoch). */
  finished: number;
  /** The thumbnail file in `<dataDirectory>/thumbs`, "" without one. */
  thumb: string;
}

export const RESULT_LIMIT = 20;
const KINDS: readonly Kind[] = ["image", "burst", "clip"];

const isSaved = (value: unknown): value is Saved => {
  const v = value as Partial<Saved>;
  return (
    typeof v === "object" && v !== null &&
    typeof v.id === "string" && KINDS.includes(v.kind as Kind) && typeof v.path === "string" && !!v.path &&
    typeof v.finished === "number" && Number.isFinite(v.finished)
  );
};
const number = (value: unknown, fallback = 0) => (typeof value === "number" && Number.isFinite(value) ? value : fallback);

/** Stored results, the valid ones, newest first, at most twenty. */
export function savedOf(stored: unknown): Saved[] {
  const list = Array.isArray((stored as { list?: unknown })?.list) ? (stored as { list: unknown[] }).list : [];
  return list
    .filter(isSaved)
    .map((v) => ({
      id: v.id, kind: v.kind, path: v.path, finished: v.finished,
      first: typeof v.first === "string" ? v.first : "",
      method: (v.method === "fast" || v.method === "precise" ? v.method : "") as Method | "",
      bytes: number(v.bytes), count: Math.max(1, Math.round(number(v.count, 1))),
      start: number(v.start), end: number(v.end, number(v.start)),
      thumb: typeof v.thumb === "string" ? basename(v.thumb) : "",
    }))
    .sort((a, b) => b.finished - a.finished)
    .slice(0, RESULT_LIMIT);
}

type ResultStore = { value: { list: unknown[] }; save(next: { list: Saved[] }): void; flush(): Promise<void> } | null;

export class Results {
  list: Saved[] = [];
  /** Ids whose file or folder is gone (checked at start and when the tab shows). */
  readonly gone = new Set<string>();
  readonly #store: ResultStore;
  #sequence = 0;
  constructor(plugin: Scope) {
    let store = null;
    try {
      store = plugin.storage.json<{ list: unknown[] }>("results", { defaults: { list: [] }, validate: (v): v is { list: unknown[] } => Array.isArray((v as { list?: unknown })?.list) });
    } catch {
      // no data directory: results last for the session
    }
    this.#store = store as unknown as ResultStore;
    this.list = savedOf(store?.value);
    this.#sequence = this.list.reduce((top, item) => Math.max(top, Number(item.id.replace(/\D/g, "")) || 0), 0);
  }

  #save() {
    this.#store?.save({ list: this.list.map((item) => ({ ...item })) });
  }

  /** A finished capture, first in the list; the twenty-first goes (returned, for its thumbnail). */
  add(fields: Omit<Saved, "id">): { saved: Saved; dropped: Saved[] } {
    const saved: Saved = { ...fields, id: `r${++this.#sequence}` };
    this.list.unshift(saved);
    const dropped = this.list.splice(RESULT_LIMIT);
    for (const item of dropped) this.gone.delete(item.id);
    this.#save();
    return { saved, dropped };
  }
  find = (id: string) => this.list.find((item) => item.id === id);
  update(item: Saved, patch: Partial<Saved>) {
    Object.assign(item, patch);
    this.#save();
  }
  remove(id: string) {
    const index = this.list.findIndex((item) => item.id === id);
    if (index < 0) return null;
    const [item] = this.list.splice(index, 1);
    this.gone.delete(id);
    this.#save();
    return item!;
  }
  clear() {
    const all = this.list;
    this.list = [];
    this.gone.clear();
    this.#save();
    return all;
  }
  /** Look again whether each file or folder exists; true when anything changed. */
  async check() {
    let changed = false;
    for (const item of this.list) {
      const there = !!(await stat(item.path).catch(() => null));
      if (there === this.gone.has(item.id)) {
        changed = true;
        if (there) this.gone.delete(item.id);
        else this.gone.add(item.id);
      }
    }
    return changed;
  }
  flush() {
    return this.#store?.flush();
  }
}

/** A finished output's size: the file's, or a burst folder's frames together. */
export async function sizeOf(path: string, kind: Kind) {
  if (kind !== "burst") return (await stat(path).catch(() => null))?.size ?? 0;
  let total = 0;
  for (const name of await readdir(path).catch(() => [] as string[])) total += (await stat(join(path, name)).catch(() => null))?.size ?? 0;
  return total;
}

/** 64×36 thumbnails (twice that for sharp HiDPI rows). */
const THUMB_W = 128;
const THUMB_H = 72;
const THUMB_TIMEOUT_MS = 10_000;
/**
 * ffmpeg's arguments for one small JPEG of a frame file or a clip (its first
 * keyframe) on stdout: one thread, as the playlist's thumbnails, so the
 * player keeps the other cores.
 */
export function thumbArgs(path: string, video: boolean) {
  return [
    "-nostdin", "-v", "error", "-threads", "1",
    ...(video ? ["-skip_frame", "nokey"] : []),
    "-i", path,
    "-map", "0:V:0", "-an", "-sn", "-dn", "-frames:v", "1", "-filter_threads", "1",
    "-vf", `scale=${THUMB_W}:${THUMB_H}:force_original_aspect_ratio=decrease:flags=bilinear,format=yuvj420p`,
    "-threads", "1", "-q:v", "5", "-f", "image2pipe", "-c:v", "mjpeg", "-",
  ];
}

/**
 * Make a result's thumbnail in `folder` (the data directory's `thumbs`); its
 * file name, or "" when ffmpeg is missing or fails (the row then shows its
 * kind's icon only). The player shows images only from the package and data
 * directories, so even a frame's own file is copied small there.
 */
export async function makeThumb(plugin: Scope, folder: string | null, item: Saved): Promise<string> {
  if (!folder) return "";
  const input = item.kind === "burst" ? item.first : item.path;
  if (!input) return "";
  try {
    if (!(await plugin.tools.available("ffmpeg"))) return "";
    const { stdout } = await plugin.tools.run("ffmpeg", thumbArgs(input, item.kind === "clip" || extname(input).toLowerCase() === ".mkv"), { timeoutMs: THUMB_TIMEOUT_MS, maxOutput: 1 << 20 });
    if (!stdout.length) return "";
    await mkdir(folder, { recursive: true });
    const name = `${item.id}-${item.finished}.jpg`;
    await writeFile(join(folder, name), stdout);
    return name;
  } catch (error) {
    console.error(`[capture] thumbnail: ${(error as Error).message}`);
    return "";
  }
}
