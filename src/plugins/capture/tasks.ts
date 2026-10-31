import { rmdir, stat } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import type { ExportJob, JobState, Scope, Snapshot } from "@vestra-player/sdk";
import { clock, extensionOf, Failure, jobProblem } from "./model.ts";
import type { JobReason, Problem, Spec } from "./model.ts";

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
}

const TERMINAL = ["completed", "failed", "cancelled"];
const RETAIN_MS = 300_000;
/**
 * A task's progress as data; ui/parts.slint `Words.progress` turns it into
 * words ("Completed", "Exporting · 3/10", "Failed: Not enough disk space").
 */
export type Progress = Problem<JobReason> & { status: string; done: number; total: number };
const IDLE: Progress = { status: "", reason: "", raw: "", done: 0, total: 0 };
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
  find = (id: string) => this.list.find((task) => task.id === id);
  retained = (task: Task) => !!task.snapshot && !!task.retainUntil && task.retainUntil > Date.now();
  /** The newest task of a kind (the burst dialog shows its progress). */
  latest = (kind: Spec["kind"]) => this.list.findLast((task) => task.spec.kind === kind);

  /** The progress of a task, or of none (an empty line). */
  progress(task: Task | undefined): Progress {
    if (!task) return IDLE;
    const { reason, raw } = task.error ?? { reason: "", raw: "" };
    return { status: task.status, reason, raw, done: task.outputs.length, total: task.spec.count };
  }

  rows() {
    return this.list
      .slice()
      .reverse()
      .map((task) => {
        const { spec } = task;
        return {
          id: task.id,
          // The row's words are in ui/tasks.slint: "Snapshot · <name>", "<range> · <progress>".
          kind: spec.kind,
          name: basename(task.destination),
          // Times are data: `00:02.000`, a clip `00:01.000–00:04.000`.
          range: spec.kind === "clip" ? `${clock(spec.start)}–${clock(spec.end!)}` : clock(spec.start),
          progress: this.progress(task),
          cancellable: !terminal(task),
          retryable: terminal(task) && task.status !== "completed",
          openable: task.outputs.length > 0,
        };
      });
  }

  /** Accept a task whose snapshot is already frozen; it keeps that input while queued. */
  add(fields: Omit<Task, "id" | "status" | "cancel" | "jobs" | "outputs">) {
    const task: Task = { ...fields, id: String(++this.#sequence), status: "queued", cancel: false, jobs: [], outputs: [] };
    this.list.push(task);
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

  #retain(task: Task) {
    if (!task.snapshot) return;
    task.retainUntil = Date.now() + RETAIN_MS;
    task.retentionTimer = setTimeout(() => void this.release(task).then(() => this.#plugin.update()), RETAIN_MS);
    task.retentionTimer.unref?.();
  }

  async #trim() {
    while (this.list.length > 20) {
      const index = this.list.findIndex(terminal);
      if (index < 0) break;
      const [old] = this.list.splice(index, 1);
      // After the first pass this runs past an await, outside SDK-invoked code: say so.
      this.#plugin.update();
      await this.release(old!);
      for (const job of old!.jobs) await job.dispose();
    }
  }

  async #pump() {
    if (this.#running || this.#stopped) return;
    this.#running = true;
    try {
      for (let task; !this.#stopped && (task = this.list.find((each) => each.status === "queued")); ) {
        task.status = "preparing";
        this.#plugin.update();
        try {
          for (let index = 0; index < task.spec.count; index++) {
            if (task.cancel || this.#stopped) break;
            const { job, state } = await this.#run(task, index);
            if (state.status === "completed" && state.outcome && "destination" in state.outcome) {
              task.outputs.push({ job, path: state.outcome.destination });
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
        ? { format: "mp4", destination, end: spec.end! }
        : format === "jpeg"
          ? { format: "jpeg", destination, sampleTime, quality: spec.quality ?? 90 }
          : { format, destination, sampleTime },
    );
    task.active = job;
    task.jobs.push(job);
    if (task.cancel || this.#stopped) await job.cancel().catch(() => {});
    const show = (state: JobState) => {
      task.status = TERMINAL.includes(state.status) ? "running" : state.status;
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
      this.#retain(task);
      if (task.directory) await rmdir(task.directory).catch(() => {});
    }
  }

  /** Open the confirmed output (a burst opens its folder) with its default application. */
  async open(task: Task) {
    const output = task.outputs.at(-1);
    if (!output) throw new Failure("noOutput");
    const state = await output.job.refresh();
    const confirmed =
      state.status === "completed" && !!state.outcome && "destination" in state.outcome && resolve(state.outcome.destination) === resolve(output.path);
    if (!confirmed || !(await stat(output.path).catch(() => null))?.isFile()) throw new Failure("outputGone");
    await this.#plugin.system.openPath(resolve(task.directory ?? output.path)).catch(() => {
      throw new Failure("cannotOpen");
    });
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
