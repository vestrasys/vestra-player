import { realpath, stat } from "node:fs/promises";
import type { Tools } from "../../sdk/plugin/index.ts";
import type { Queue, Row } from "./queue.ts";

/** Runs ffprobe with `args` and resolves its stdout; rejects on failure, timeout or abort. */
export type ProbeRunner = (args: string[], options: { signal: AbortSignal; timeoutMs: number }) => Promise<string>;

export const PROBE_ARGS = ["-v", "error", "-show_entries", "format=duration", "-of", "json"];
export const PROBE_LIMIT = 2;
export const PROBE_TIMEOUT_MS = 10_000;
/** ffprobe's JSON for one duration is tiny; more than this is not an answer. */
export const PROBE_OUTPUT_LIMIT = 64 * 1024;
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
 */
export class Prober {
  /** realpath + mtime -> seconds (null: ffprobe answered without a duration); insertion order = LRU order. */
  private cache = new Map<string, number | null>();
  /** The cache changed since the last `export()`. */
  dirty = false;
  private waiting: string[] = [];
  private running = new Map<string, AbortController>();

  private queue: Queue;
  private runner: ProbeRunner;
  private changed: () => void;
  constructor(queue: Queue, runner: ProbeRunner, changed: () => void) {
    this.queue = queue;
    this.runner = runner;
    this.changed = changed;
  }

  /** Queue every local row not probed yet. URL rows are never probed. */
  scan() {
    for (const row of this.queue.rows)
      if (row.kind === "file" && row.probe === null) {
        row.probe = "pending";
        this.waiting.push(row.rowId);
      }
    this.pump();
  }
  /** The row left the list: drop it from the wait list and kill its process. */
  cancel(rowId: string) {
    this.waiting = this.waiting.filter((id) => id !== rowId);
    const running = this.running.get(rowId);
    running?.abort();
    this.running.delete(rowId);
    // The slot is free now, not when the ended process is reaped.
    if (running) this.pump();
  }
  cancelAll() {
    this.waiting = [];
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
  /** Processes currently running (for tests). */
  get active() {
    return this.running.size;
  }

  private pump() {
    while (this.running.size < PROBE_LIMIT && this.waiting.length) {
      const rowId = this.waiting.shift()!;
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
    if (value !== null) row.probed = value;
    row.probe = row.probed === null ? "failed" : "done";
    this.changed();
  }
}
