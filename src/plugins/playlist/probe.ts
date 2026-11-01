import { realpath, stat } from "node:fs/promises";
import type { Tools } from "@vestra-player/sdk";
import type { Queue, Row } from "./queue.ts";

/** Runs ffprobe with `args` and resolves its stdout; rejects on failure, timeout or abort. */
export type ProbeRunner = (args: string[], options: { signal: AbortSignal; timeoutMs: number }) => Promise<string>;

export const PROBE_ARGS = ["-v", "error", "-show_entries", "format=duration", "-of", "json"];
export const PROBE_LIMIT = 2;
export const PROBE_TIMEOUT_MS = 10_000;
/** ffprobe's JSON for one duration is tiny; more than this is not an answer. */
export const PROBE_OUTPUT_LIMIT = 64 * 1024;
/** Answers within this long reach the list as one update. */
export const PROBE_NOTIFY_MS = 50;
/** C17: entries kept in `probe-cache.json` (least recently used dropped first). */
export const CACHE_LIMIT = 20_000;

/** ffprobe through `plugin.tools` (lookup, no shell, timeout, abort and output bound). */
export const toolRunner =
  (tools: Tools): ProbeRunner =>
  async (args, { signal, timeoutMs }) => {
    const { stdout } = await tools.run("ffprobe", args, { signal, timeoutMs, maxOutput: PROBE_OUTPUT_LIMIT });
    return Buffer.from(stdout).toString("utf8");
  };

/** Seconds from ffprobe's JSON, or null when it reports none. */
export function parseDuration(output: string): number | null {
  const value = Number((JSON.parse(output) as { format?: { duration?: unknown } }).format?.duration);
  return Number.isFinite(value) && value > 0 ? value : null;
}

/**
 * Reads durations of local rows that have not played: at most `PROBE_LIMIT`
 * processes, cached by realpath + mtime, aborted when their row goes away.
 * Failures only mark the row unknown (`--:--`); they are never announced.
 * Once `want` names the rows on screen, only those are read (as PotPlayer and
 * mpv: adding thousands of files never starts thousands of ffprobe processes;
 * a row that plays gets its length from the player).
 */
export class Prober {
  /** realpath + mtime -> seconds (null: ffprobe answered without a duration); insertion order = LRU order. */
  private cache = new Map<string, number | null>();
  /** The cache changed since the last `export()`. */
  dirty = false;
  /** Rows to probe, in list order when queued (a Set: removing thousands stays linear). */
  private waiting = new Set<string>();
  /** The rows to read (those on screen); null: every row. */
  private wanted: ReadonlySet<string> | null = null;
  private running = new Map<string, AbortController>();
  private notify: ReturnType<typeof setTimeout> | null = null;
  private notifiedAt = Number.NEGATIVE_INFINITY;

  private queue: Queue;
  private runner: ProbeRunner;
  private changed: () => void;
  constructor(queue: Queue, runner: ProbeRunner, changed: () => void) {
    this.queue = queue;
    this.runner = runner;
    this.changed = changed;
  }

  /** Queue the wanted local rows not probed yet. URL rows are never probed. */
  scan() {
    const rows = this.wanted ? [...this.wanted].map((id) => this.queue.row(id)) : this.queue.rows;
    for (const row of rows)
      if (row && row.kind === "file" && row.probe === null) {
        row.probe = "pending";
        this.waiting.add(row.rowId);
      }
    this.pump();
  }
  /** The row left the list: drop it from the wait list and kill its process. */
  cancel(rowId: string) {
    this.waiting.delete(rowId);
    const running = this.running.get(rowId);
    running?.abort();
    this.running.delete(rowId);
    // The slot is free now, not when the ended process is reaped.
    if (running) this.pump();
  }
  cancelAll() {
    this.waiting.clear();
    if (this.notify) clearTimeout(this.notify);
    this.notify = null;
    for (const controller of this.running.values()) controller.abort();
    this.running.clear();
  }
  /** The cache as saved in `probe-cache.json`. */
  export(): [string, number | null][] {
    this.dirty = false;
    return [...this.cache];
  }
  import(entries: unknown) {
    if (!Array.isArray(entries)) return;
    for (const entry of entries.slice(-CACHE_LIMIT))
      if (Array.isArray(entry) && typeof entry[0] === "string" && (entry[1] === null || typeof entry[1] === "number"))
        this.cache.set(entry[0], entry[1]);
  }
  private remember(key: string, value: number | null) {
    this.cache.delete(key);
    this.cache.set(key, value);
    while (this.cache.size > CACHE_LIMIT) this.cache.delete(this.cache.keys().next().value!);
    this.dirty = true;
  }
  /**
   * Read only these rows (the ones on screen): rows scrolled away that have not
   * started wait again (unasked); a running ffprobe finishes.
   */
  want(rowIds: readonly string[]) {
    const wanted = new Set(rowIds);
    this.wanted = wanted;
    for (const id of [...this.waiting])
      if (!wanted.has(id)) {
        this.waiting.delete(id);
        const row = this.queue.row(id);
        if (row?.probe === "pending") row.probe = null;
      }
    this.scan();
  }
  /** Processes currently running (for tests). */
  get active() {
    return this.running.size;
  }

  private pump() {
    while (this.running.size < PROBE_LIMIT && this.waiting.size) {
      const rowId = this.waiting.values().next().value!;
      this.waiting.delete(rowId);
      const row = this.queue.row(rowId);
      if (!row) continue;
      const controller = new AbortController();
      this.running.set(rowId, controller);
      void this.probe(row, controller.signal).finally(() => {
        if (this.running.get(rowId) === controller) this.running.delete(rowId);
        this.pump();
      });
    }
  }

  private async probe(row: Row, signal: AbortSignal) {
    let value: number | null;
    try {
      const real = await realpath(row.source);
      // Restored rows (C17) learn their realpath here.
      if (!signal.aborted && !this.queue.rows.some((other) => other !== row && other.kind === "file" && other.key === real))
        row.key = real;
      const key = `${real}\n${(await stat(real)).mtimeMs}`;
      const cached = this.cache.get(key);
      if (cached !== undefined) {
        value = cached;
        this.remember(key, cached);
      } else {
        if (signal.aborted) return;
        const output = await this.runner([...PROBE_ARGS, real], { signal, timeoutMs: PROBE_TIMEOUT_MS });
        value = parseDuration(output);
        // Only answers ffprobe gave are cached; a missing tool or timeout is retried on the next add.
        this.remember(key, value);
      }
    } catch {
      value = null;
    }
    if (signal.aborted) return;
    // A length from a list file or the last run (C17/C18) stays when ffprobe has none.
    if (value !== null && row.probed !== value) {
      row.probed = value;
      this.queue.touch();
    }
    row.probe = row.probed === null ? "failed" : "done";
    this.answered();
  }
  /**
   * Tell the list: at once for the first answer after a quiet spell and for the last
   * one outstanding, else once per `PROBE_NOTIFY_MS` (3000 cached answers arrive
   * within milliseconds; each update resends the rows on screen).
   */
  private answered() {
    const now = performance.now();
    // Runs that already answered may still be in `running` until their promise settles.
    const last =
      this.waiting.size === 0 && [...this.running.keys()].every((id) => this.queue.row(id)?.probe !== "pending");
    if (this.notify && !last) return;
    if (last || now - this.notifiedAt >= PROBE_NOTIFY_MS) {
      if (this.notify) clearTimeout(this.notify);
      this.notify = null;
      this.notifiedAt = now;
      return this.changed();
    }
    this.notify = setTimeout(() => {
      this.notify = null;
      this.notifiedAt = performance.now();
      this.changed();
    }, PROBE_NOTIFY_MS - (now - this.notifiedAt));
  }
}
