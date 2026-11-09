import { rmdir } from "node:fs/promises";
import { join } from "node:path";
import type { ExportJob, JobState, Scope, Snapshot } from "@vestra-player/sdk";
import { extensionOf, Failure, jobProblem } from "./model.ts";
import type { JobReason, Problem, Spec } from "./model.ts";

/** An accepted capture: one frozen snapshot, one or more sequential still exports (a frame, a burst's frames). */
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
}

const TERMINAL = ["completed", "failed", "cancelled"];
/** At most one running and four queued tasks. */
export const QUEUE_LIMIT = 5;
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

/** One running and up to four queued tasks; a task leaves the list once it ended and was reported. */
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

  /** Accept a task whose snapshot is already frozen; it keeps that input while queued. */
  add(fields: Omit<Task, "id" | "status" | "cancel" | "jobs" | "outputs">) {
    const task: Task = { ...fields, id: String(++this.#sequence), status: "queued", cancel: false, jobs: [], outputs: [] };
    this.list.push(task);
    void this.#pump();
    return task;
  }

  /** Take an ended task off the list and release what it holds. */
  async forget(task: Task) {
    if (!terminal(task)) return;
    const index = this.list.indexOf(task);
    if (index >= 0) this.list.splice(index, 1);
    const snapshot = task.snapshot;
    task.snapshot = undefined;
    await snapshot?.dispose();
    for (const job of task.jobs) await job.dispose().catch(() => {});
  }

  async #pump() {
    if (this.#running || this.#stopped) return;
    this.#running = true;
    try {
      for (let task; !this.#stopped && (task = this.list.find((each) => each.status === "queued")); ) {
        task.status = "running";
        try {
          for (let index = 0; index < task.spec.count; index++) {
            if (task.cancel || this.#stopped) break;
            const { job, state } = await this.#run(task, index);
            if (state.status === "completed" && state.outcome && "destination" in state.outcome) task.outputs.push({ job, path: state.outcome.destination });
            else if (state.status === "failed")
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
        if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
        this.#finished(task);
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
      format === "jpeg" ? { format: "jpeg", destination, sampleTime, quality: spec.quality ?? 90 } : { format, destination, sampleTime },
    );
    task.active = job;
    task.jobs.push(job);
    if (task.cancel || this.#stopped) await job.cancel().catch(() => {});
    try {
      return { job, state: await job.done };
    } finally {
      task.active = undefined;
    }
  }

  /** Deactivation: the SDK cancels jobs and releases snapshots; remove empty owned folders. */
  async stop() {
    this.#stopped = true;
    for (const task of this.list) {
      task.cancel = true;
      if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
    }
  }
}
