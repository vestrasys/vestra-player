import { rmdir } from "node:fs/promises";
import { join } from "node:path";
import type { ExportJob, JobState, Scope, Snapshot } from "@vestra-player/sdk";
import { extensionOf, Failure, jobProblem } from "./model.ts";
import type { JobReason, Problem, Spec } from "./model.ts";
import { actualRange, clipExport, jobProgress } from "./seams.ts";

/** An accepted export: one frozen snapshot, one or more sequential jobs. */
export interface Task {
  id: string;
  spec: Spec;
  sourceId: number;
  source: string;
  snapshot?: Snapshot;
  destination: string;
  /** The unique burst folder this plugin created. */
  directory?: string;
  status: string;
  /** Why it failed. */
  error?: Problem<JobReason>;
  cancel: boolean;
  jobs: ExportJob[];
  /** `path`: the file the host confirmed (JPEG/BMP are converted by the player before it publishes). */
  outputs: { job: ExportJob; path: string }[];
  active?: ExportJob;
  retainUntil?: number;
  retentionTimer?: ReturnType<typeof setTimeout>;
  /** When its first job started running, and when it ended (ms since the epoch). */
  started?: number;
  ended?: number;
  /** The running job's progress in seconds of media (R4); `null` while unknown. */
  progress: { done: number; total: number } | null;
  /** A finished clip's range as written (R2; the range asked for until then). */
  actual?: { start: number; end: number };
}

const TERMINAL = ["completed", "failed", "cancelled"];
const RETAIN_MS = 300_000;
/** At most one running and four queued tasks; twenty in the list. */
export const QUEUE_LIMIT = 5;
export const HISTORY_LIMIT = 20;
/**
 * A task's progress as data; ui/parts.slint `Words.progress` turns it into
 * words ("Completed", "Exporting · 3/10", "Failed: Not enough disk space").
 */
export type Progress = Problem<JobReason> & { status: string; done: number; total: number };
export const terminal = (task: Task) => TERMINAL.includes(task.status);

/** A host job failure as one of the plugin's reasons; the code and message go to the log. */
export function jobError(error: { code: string; message: string }): JobReason {
  console.error(`[capture] export failed: ${error.code}: ${error.message}`);
  switch (error.code) {
    case "space":
      return "space";
    case "exists":
      return "exists";
    case "permission":
      return "permission";
    case "encoder":
    case "unsupported":
      return "encoder";
    default:
      return "exportFailed";
  }
}

/** One running and up to four queued tasks; history of twenty; failed input kept five minutes. */
export class Tasks {
  readonly list: Task[] = [];
  readonly #plugin: Scope;
  readonly #finished: (task: Task) => void;
  #sequence = 0;
  #running = false;
  #stopped = false;
  constructor(plugin: Scope, finished: (task: Task) => void) {
    this.#plugin = plugin;
    this.#finished = finished;
  }

  active = () => this.list.filter((task) => !terminal(task));
  full = () => this.active().length >= QUEUE_LIMIT;
  find = (id: string) => this.list.find((task) => task.id === id);
  retained = (task: Task) => !!task.snapshot && !!task.retainUntil && task.retainUntil > Date.now();
  /** The task running now, if any. */
  running = () => this.list.find((task) => !terminal(task) && task.status !== "queued");
  /** How many wait behind it. */
  queued = () => this.list.filter((task) => task.status === "queued").length;

  /** The progress of a task as data. */
  progressOf(task: Task): Progress {
    const { reason, raw } = task.error ?? { reason: "", raw: "" };
    return { status: task.status, reason, raw, done: task.outputs.length, total: task.spec.count };
  }

  /** Accept a task whose snapshot is already frozen; it keeps that input while queued. */
  add(fields: Omit<Task, "id" | "status" | "cancel" | "jobs" | "outputs" | "progress">) {
    const task: Task = { ...fields, id: String(++this.#sequence), status: "queued", cancel: false, jobs: [], outputs: [], progress: null };
    this.list.push(task);
    // Accepted past the caller's awaits (the snapshot, a free name): the row is new outside SDK-invoked code.
    this.#plugin.update();
    void this.#trim();
    void this.#pump();
    return task;
  }

  /** Hand a retained snapshot to a retry; the old task no longer owns it. */
  take(task: Task): Snapshot {
    if (!this.retained(task)) throw new Failure("expired");
    const snapshot = task.snapshot!;
    clearTimeout(task.retentionTimer);
    task.snapshot = task.retainUntil = task.retentionTimer = undefined;
    return snapshot;
  }

  async release(task: Task) {
    clearTimeout(task.retentionTimer);
    const snapshot = task.snapshot;
    task.snapshot = task.retainUntil = task.retentionTimer = undefined;
    await snapshot?.dispose();
  }

  /** Take an ended task off the list (Remove from List, Clear Finished, a completed task once it is a result). */
  async forget(task: Task) {
    if (!terminal(task)) return;
    const index = this.list.indexOf(task);
    if (index >= 0) this.list.splice(index, 1);
    await this.release(task);
    for (const job of task.jobs) await job.dispose().catch(() => {});
  }

  #retain(task: Task) {
    if (!task.snapshot) return;
    task.retainUntil = Date.now() + RETAIN_MS;
    task.retentionTimer = setTimeout(() => void this.release(task).then(() => this.#plugin.update()), RETAIN_MS);
    task.retentionTimer.unref?.();
  }

  async #trim() {
    while (this.list.length > HISTORY_LIMIT) {
      const old = this.list.find(terminal);
      if (!old) break;
      await this.forget(old);
      // Past an await, outside SDK-invoked code: say so.
      this.#plugin.update();
    }
  }

  async #pump() {
    if (this.#running || this.#stopped) return;
    this.#running = true;
    try {
      for (let task; !this.#stopped && (task = this.list.find((each) => each.status === "queued")); ) {
        task.status = "preparing";
        task.started = Date.now();
        this.#plugin.update();
        try {
          for (let index = 0; index < task.spec.count; index++) {
            if (task.cancel || this.#stopped) break;
            const { job, state } = await this.#run(task, index);
            if (state.status === "completed" && state.outcome && "destination" in state.outcome) {
              task.outputs.push({ job, path: state.outcome.destination });
              if (task.spec.kind === "clip") task.actual = actualRange(state, { start: task.spec.start, end: task.spec.end! });
              // Past the job's await: the row's "n/total" changed outside SDK-invoked code.
              this.#plugin.update();
            } else if (state.status === "failed")
              throw new Failure(state.outcome && "error" in state.outcome ? jobError(state.outcome.error) : jobError({ code: "unknown", message: "no error outcome" }));
            else if (state.status === "cancelled") {
              task.cancel = true;
              break;
            } else throw new Failure(jobError({ code: "unknown", message: "completed without a destination" }));
          }
          task.status = task.outputs.length === task.spec.count ? "completed" : "cancelled";
        } catch (error) {
          task.status = "failed";
          task.error = error instanceof Error ? jobProblem(error) : { reason: "exportFailed", raw: "" };
        }
        task.ended = Date.now();
        task.progress = null;
        if (task.status === "completed") await this.release(task);
        else this.#retain(task);
        if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
        this.#finished(task);
        this.#plugin.update();
      }
    } finally {
      this.#running = false;
    }
  }

  async #run(task: Task, index: number): Promise<{ job: ExportJob; state: JobState }> {
    if (!task.snapshot) throw new Failure("released");
    const { spec } = task;
    const sampleTime = spec.start + index * spec.step;
    const format = spec.format ?? "png";
    const destination =
      spec.kind === "burst" ? join(task.destination, `${String(index + 1).padStart(3, "0")}-${sampleTime.toFixed(3)}s${extensionOf(format)}`) : task.destination;
    // F07: the player converts JPEG/BMP from its checked PNG and publishes only the converted file.
    const job = await task.snapshot.export(
      spec.kind === "clip"
        ? clipExport(destination, spec.end!, spec.method ?? "precise", spec.encode)
        : format === "jpeg"
          ? { format: "jpeg", destination, sampleTime, quality: spec.quality ?? 90 }
          : { format, destination, sampleTime },
    );
    task.active = job;
    task.jobs.push(job);
    if (task.cancel || this.#stopped) await job.cancel().catch(() => {});
    const show = (state: JobState) => {
      // A job the player has not started yet is still this task's preparation (the task left the plugin's queue).
      task.status = TERMINAL.includes(state.status) ? "running" : state.status === "queued" ? "preparing" : state.status;
      task.progress = jobProgress(state);
    };
    show(job.state);
    // Past the export's await: the row's status changed outside SDK-invoked code.
    this.#plugin.update();
    const watching = job.onChange(show);
    try {
      return { job, state: await job.done };
    } finally {
      task.active = undefined;
      await watching.dispose();
    }
  }

  async cancel(task: Task) {
    if (terminal(task)) return;
    task.cancel = true;
    if (task.active) {
      task.status = "cancelling";
      await task.active.cancel().catch((error) => console.error(`[capture] cancel: ${error}`));
    } else if (task.status === "queued") {
      task.status = "cancelled";
      task.ended = Date.now();
      this.#retain(task);
      if (task.directory) await rmdir(task.directory).catch(() => {});
    }
  }

  /** Deactivation: the SDK cancels jobs and releases snapshots; remove empty owned folders. */
  async stop() {
    this.#stopped = true;
    for (const task of this.list) {
      task.cancel = true;
      clearTimeout(task.retentionTimer);
      if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
    }
  }
}
