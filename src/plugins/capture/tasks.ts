import { spawn } from "node:child_process";
import { rmdir, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import type { ExportJob, JobState, Scope, Snapshot } from "../../sdk/plugin/index.ts";
import { message } from "./model.ts";
import type { Spec } from "./model.ts";

/** An accepted export: one frozen snapshot, one or more sequential jobs. */
export interface Task {
  id: string;
  spec: Spec;
  sourceId: number;
  source: string;
  created: string;
  snapshot?: Snapshot;
  destination: string;
  /** The unique burst folder this plugin created. */
  directory?: string;
  status: string;
  detail: string;
  cancel: boolean;
  jobs: ExportJob[];
  outputs: { job: ExportJob; path: string }[];
  active?: ExportJob;
  retainUntil?: number;
  retentionTimer?: ReturnType<typeof setTimeout>;
  facts: string[];
}

const TERMINAL = ["completed", "failed", "cancelled"];
const RETAIN_MS = 300_000;
export const terminal = (task: Task) => TERMINAL.includes(task.status);

/** One running and up to four queued tasks; history of twenty; failed input kept five minutes. */
export class Tasks {
  readonly list: Task[] = [];
  readonly #plugin: Scope;
  readonly #say: (text: string) => void;
  #sequence = 0;
  #running = false;
  #stopped = false;
  constructor(plugin: Scope, say: (text: string) => void) {
    this.#plugin = plugin;
    this.#say = say;
  }

  active = () => this.list.filter((task) => !terminal(task));
  find = (id: string) => this.list.find((task) => task.id === id);
  retained = (task: Task) => !!task.snapshot && !!task.retainUntil && task.retainUntil > Date.now();

  rows() {
    return this.list
      .slice()
      .reverse()
      .map((task) => ({
        id: task.id,
        summary: `#${task.id} · ${task.source}\n${task.created} · ${task.spec.kind} · ${task.spec.processed ? "processed" : "raw"} · ${task.status}\n${task.spec.start.toFixed(3)}${task.spec.end === undefined ? "" : `–${task.spec.end}`}s · ${task.outputs.length}/${task.spec.count} published\n${task.detail}\n${task.destination}${terminal(task) && task.status !== "completed" ? (task.retainUntil ? `\nRetry input expires ${new Date(task.retainUntil).toLocaleTimeString()}` : "\nInput released; retry requires current source.") : ""}${task.facts.length ? `\n${task.facts.at(-1)}` : ""}`,
        cancellable: !terminal(task),
        retryable: terminal(task) && task.status !== "completed",
        retained: this.retained(task),
        openable: task.outputs.length > 0,
      }));
  }

  /** Accept a task whose snapshot is already frozen; it keeps that input while queued. */
  add(fields: Omit<Task, "id" | "status" | "cancel" | "jobs" | "outputs" | "facts">) {
    const task: Task = { ...fields, id: String(++this.#sequence), status: "queued", cancel: false, jobs: [], outputs: [], facts: [] };
    this.list.push(task);
    void this.#trim();
    void this.#pump();
    return task;
  }

  /** Hand a retained snapshot to a retry; the old task no longer owns it. */
  take(task: Task): Snapshot {
    if (!this.retained(task)) throw new Error("Retained snapshot expired during confirmation.");
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
            if (state.facts) {
              const f = state.facts;
              const size = task.snapshot?.presentation ? `${task.snapshot.presentation.width}×${task.snapshot.presentation.height}` : "source display dimensions";
              task.facts.push(`Video stream ${f.videoStreamIndex}; audio ${f.hasAudio}; ${f.audioTiming}; preroll ${f.preroll ?? "none"}; descriptor ${f.descriptorVersion}; FFmpeg ${f.ffmpegVersion}; range ${JSON.stringify(f.range)}; ${size}; SDR 8-bit RGB PNG / H.264+AAC MP4`);
            }
            if (state.status === "completed" && state.outcome && "destination" in state.outcome) task.outputs.push({ job, path: state.outcome.destination });
            else if (state.status === "failed")
              throw new Error(state.outcome && "error" in state.outcome ? `${state.outcome.error.code}: ${state.outcome.error.message}` : "Worker failed without an error outcome.");
            else if (state.status === "cancelled") {
              task.cancel = true;
              break;
            } else throw new Error("Worker completed without a confirmed output destination.");
          }
          task.status = task.outputs.length === task.spec.count ? "completed" : "cancelled";
          task.detail = `${task.outputs.length}/${task.spec.count} confirmed published outputs. ${task.outputs.map((output) => output.path).join("; ")}`;
        } catch (error) {
          task.status = "failed";
          task.detail = message(error);
        }
        if (task.status === "completed") await this.release(task);
        else this.#retain(task);
        if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
        this.#say(`#${task.id} · ${task.source}: ${task.status}. ${task.detail}`);
      }
    } finally {
      this.#running = false;
    }
  }

  async #run(task: Task, index: number): Promise<{ job: ExportJob; state: JobState }> {
    if (!task.snapshot) throw new Error("Snapshot is no longer retained.");
    const { spec } = task;
    const sampleTime = spec.start + index * spec.step;
    const destination = spec.kind === "burst" ? join(task.destination, `${String(index + 1).padStart(3, "0")}-${sampleTime.toFixed(3)}s.png`) : task.destination;
    const job = await task.snapshot.export(spec.kind === "clip" ? { format: "mp4", destination, end: spec.end! } : { format: "png", destination, sampleTime });
    task.active = job;
    task.jobs.push(job);
    if (task.cancel || this.#stopped) await job.cancel().catch(() => {});
    const show = (state: JobState) => {
      task.status = TERMINAL.includes(state.status) ? "running" : state.status;
      task.detail = `${spec.kind === "burst" ? `Image ${index + 1}/${spec.count} at ${sampleTime.toFixed(3)}s · ` : ""}${state.status}${state.publicationStarted ? " · publication started" : ""}`;
    };
    show(job.state);
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
      const job = task.active;
      task.status = "cancelling";
      const result = await job.cancel().catch((error) => {
        task.detail = `Cancellation result unknown: ${message(error)}; querying job until terminal.`;
        return null;
      });
      if (result) task.detail = result === "tooLate" ? "Publication already started; cancellation may be too late. Waiting for actual outcome." : "Cancellation requested; waiting for backend terminal state.";
    } else if (task.status === "queued") {
      task.status = "cancelled";
      task.detail = "Cancelled before worker start.";
      this.#retain(task);
      if (task.directory) await rmdir(task.directory).catch(() => {});
    } else task.detail = "Cancellation requested while preparing; waiting for job identity/outcome.";
  }

  async query(task: Task) {
    const observed = await Promise.all(
      task.jobs.map(async (job, index) => {
        const state = await job.refresh();
        return `Worker ${index + 1}: ${state.status}${state.publicationStarted ? " (publication started)" : ""}`;
      }),
    );
    return `#${task.id} · ${task.source}: ${task.detail}\n${observed.join("; ")}\n${task.facts.join("\n")}\n${task.outputs.map((output) => output.path).join("\n")}`;
  }

  /** Open the confirmed output (a burst opens its folder) with the system opener. */
  async open(task: Task) {
    const output = task.outputs.at(-1);
    if (!output) throw new Error("No confirmed output exists for this task.");
    const state = await output.job.refresh();
    if (state.status !== "completed" || !state.outcome || !("destination" in state.outcome) || resolve(state.outcome.destination) !== resolve(output.path) || !(await stat(output.path)).isFile())
      throw new Error("Published output is missing or no longer confirmed by its original job.");
    const path = resolve(task.directory ?? output.path);
    const program = process.platform === "darwin" ? "open" : process.platform === "win32" ? "explorer.exe" : "xdg-open";
    const args = process.platform === "darwin" ? ["--", path] : [path];
    await new Promise<void>((done, reject) => {
      const child = spawn(program, args, { shell: false, stdio: "ignore" });
      child.once("error", reject);
      child.once("exit", (code) => (code === 0 ? done() : reject(new Error(`System opener exited ${code}`))));
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
