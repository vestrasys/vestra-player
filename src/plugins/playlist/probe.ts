import { spawn } from "node:child_process";
import { realpath, stat } from "node:fs/promises";
import type { Queue, Row } from "./queue.ts";

/** Runs ffprobe with `args` and resolves its stdout; rejects on failure, timeout or abort. */
export type ProbeRunner = (args: string[], options: { signal: AbortSignal; timeoutMs: number }) => Promise<string>;

export const PROBE_ARGS = ["-v", "error", "-show_entries", "format=duration", "-of", "json"];
export const PROBE_LIMIT = 2;
export const PROBE_TIMEOUT_MS = 10_000;
/** C17: entries kept in `probe-cache.json` (least recently used dropped first). */
export const CACHE_LIMIT = 20_000;

/** `VESTRA_FFPROBE` or `ffprobe` on PATH; no shell, bounded output and time. */
export function createProbeRunner(executable = process.env.VESTRA_FFPROBE || "ffprobe"): ProbeRunner {
  return (args, { signal, timeoutMs }) =>
    new Promise((resolve, reject) => {
      if (signal.aborted) return reject(new Error("cancelled"));
      const child = spawn(executable, args, { shell: false, stdio: ["ignore", "pipe", "ignore"] });
      let stdout = "",
        failure: Error | undefined;
      const stop = (error: Error) => {
        failure ??= error;
        child.kill("SIGKILL");
      };
      const cancel = () => stop(new Error("cancelled"));
      signal.addEventListener("abort", cancel, { once: true });
      const timeout = setTimeout(() => stop(new Error("ffprobe timed out")), timeoutMs);
      child.stdout.on("data", (chunk: Buffer) => {
        stdout += chunk.toString("utf8");
        if (stdout.length > 64 * 1024) stop(new Error("ffprobe output exceeded the limit"));
      });
      child.on("error", (error) => (failure ??= error));
      child.on("close", (code) => {
        clearTimeout(timeout);
        signal.removeEventListener("abort", cancel);
        if (failure) reject(failure);
        else if (code !== 0) reject(new Error(`ffprobe exited with ${code}`));
        else resolve(stdout);
      });
    });
}

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
    this.running.get(rowId)?.abort();
    this.running.delete(rowId);
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
