import { lstat, rename, rm, rmdir, stat } from "node:fs/promises";
import { basename, dirname, extname, join, resolve } from "node:path";
import type { ExportJob, JobState, Scope, Snapshot } from "../../sdk/plugin/index.ts";
import { clock, extensionOf, KIND_LABEL, message, uniqueFile } from "./model.ts";
import type { Spec } from "./model.ts";
import { system } from "./system.ts";

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
  /** Chinese one-line failure reason. */
  error?: string;
  cancel: boolean;
  jobs: ExportJob[];
  /** `published`: the file the host confirmed; `path`: the final file (differs after a JPEG/BMP conversion). */
  outputs: { job: ExportJob; path: string; published: string }[];
  /** F07: a conversion failed and the PNG was kept instead (for the OSD). */
  fallback?: boolean;
  active?: ExportJob;
  retainUntil?: number;
  retentionTimer?: ReturnType<typeof setTimeout>;
}

const TERMINAL = ["completed", "failed", "cancelled"];
const RETAIN_MS = 300_000;
const STATUS: Record<string, string> = {
  queued: "排队中",
  preparing: "准备中",
  running: "导出中",
  cancelling: "正在取消",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};
export const terminal = (task: Task) => TERMINAL.includes(task.status);

/** A host job failure in one Chinese sentence; the code and message go to the log. */
export function jobError(error: { code: string; message: string }) {
  console.error(`[capture] export failed: ${error.code}: ${error.message}`);
  switch (error.code) {
    case "space":
      return "磁盘空间不足";
    case "exists":
      return "目标文件已存在";
    case "permission":
      return "没有写入权限";
    case "encoder":
    case "unsupported":
      return "无法编码此媒体";
    default:
      return "导出失败";
  }
}

/** Open a file or folder with the system opener. */
export const reveal = (path: string) => system.reveal(path);

/** Whether a path is free (nothing, not even a dangling link, is there). */
async function free(path: string) {
  return lstat(path).then(
    () => false,
    (error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return true;
      throw error;
    },
  );
}

/**
 * F07: turn the PNG the host published into the final JPEG/BMP file. On any
 * failure the PNG is kept, renamed to the final name with `.png`.
 */
export async function convert(published: string, final: string, spec: Spec): Promise<{ path: string; fallback: boolean }> {
  const format = spec.format ?? "png";
  if (format === "png") return { path: published, fallback: false };
  let target: string | undefined;
  try {
    target = (await free(final)) ? final : await uniqueFile(final);
    await system.convertImage(published, target, format, spec.quality ?? 90);
    // sips reports some failures only on stderr with exit status 0: check the result.
    if (!(await stat(target).catch(() => null))?.size) throw new Error("no output");
    await rm(published, { force: true });
    return { path: target, fallback: false };
  } catch (error) {
    console.error(`[capture] cannot convert to ${format}: ${message(error)}`);
    if (target) await rm(target, { force: true }); // a partial file of our own choosing
    const png = await uniqueFile(join(dirname(final), `${basename(final, extname(final))}.png`));
    await rename(published, png);
    return { path: png, fallback: true };
  }
}

/** Where the PNG export goes before conversion: a hidden name next to the final file. */
const staging = (final: string) => uniqueFile(join(dirname(final), `.${basename(final, extname(final))}.vestra-tmp.png`));

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

  /** `已完成`, `导出中 · 3/10`, `失败：磁盘空间不足`. */
  progress(task: Task) {
    const count = task.spec.count > 1 ? ` · ${task.outputs.length}/${task.spec.count}` : "";
    return `${STATUS[task.status] ?? task.status}${task.error ? `：${task.error}` : count}`;
  }

  rows() {
    return this.list
      .slice()
      .reverse()
      .map((task) => {
        const { spec } = task;
        const range = spec.kind === "clip" ? `${clock(spec.start)}–${clock(spec.end!)}` : clock(spec.start);
        return {
          id: task.id,
          title: `${KIND_LABEL[spec.kind]} · ${basename(task.destination)}`,
          detail: `${range} · ${this.progress(task)}`,
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
    if (!this.retained(task)) throw new Error("保留的画面已过期，请用当前画面重试。");
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
            const { job, state, final } = await this.#run(task, index);
            if (state.status === "completed" && state.outcome && "destination" in state.outcome) {
              const published = state.outcome.destination;
              const { path, fallback } = await convert(published, final, task.spec).catch((error) => {
                console.error(`[capture] cannot keep the frame: ${message(error)}`);
                throw new Error("无法保存图片");
              });
              if (fallback) task.fallback = true;
              task.outputs.push({ job, path, published });
            } else if (state.status === "failed")
              throw new Error(state.outcome && "error" in state.outcome ? jobError(state.outcome.error) : jobError({ code: "unknown", message: "no error outcome" }));
            else if (state.status === "cancelled") {
              task.cancel = true;
              break;
            } else throw new Error(jobError({ code: "unknown", message: "completed without a destination" }));
          }
          task.status = task.outputs.length === task.spec.count ? "completed" : "cancelled";
        } catch (error) {
          task.status = "failed";
          task.error = error instanceof Error ? error.message : "导出失败";
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

  async #run(task: Task, index: number): Promise<{ job: ExportJob; state: JobState; final: string }> {
    if (!task.snapshot) throw new Error("保留的画面已释放。");
    const { spec } = task;
    const sampleTime = spec.start + index * spec.step;
    const extension = extensionOf(spec.format ?? "png");
    const final = spec.kind === "burst" ? join(task.destination, `${String(index + 1).padStart(3, "0")}-${sampleTime.toFixed(3)}s${extension}`) : task.destination;
    // JPEG/BMP: export a PNG under a hidden temporary name, convert after the host confirms it.
    const destination = spec.kind !== "clip" && extension !== ".png" ? await staging(final) : final;
    const job = await task.snapshot.export(spec.kind === "clip" ? { format: "mp4", destination, end: spec.end! } : { format: "png", destination, sampleTime });
    task.active = job;
    task.jobs.push(job);
    if (task.cancel || this.#stopped) await job.cancel().catch(() => {});
    const show = (state: JobState) => {
      task.status = TERMINAL.includes(state.status) ? "running" : state.status;
    };
    show(job.state);
    const watching = job.onChange(show);
    try {
      return { job, state: await job.done, final };
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

  /** Open the confirmed output (a burst opens its folder) with the system opener. */
  async open(task: Task) {
    const output = task.outputs.at(-1);
    if (!output) throw new Error("这个任务还没有输出文件。");
    const state = await output.job.refresh();
    const confirmed =
      state.status === "completed" && !!state.outcome && "destination" in state.outcome && resolve(state.outcome.destination) === resolve(output.published);
    if (!confirmed || !(await stat(output.path).catch(() => null))?.isFile()) throw new Error("输出文件已不存在。");
    await reveal(resolve(task.directory ?? output.path));
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
