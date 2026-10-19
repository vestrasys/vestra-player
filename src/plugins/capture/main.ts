import type { Host, Json } from "../../sdk/index.ts";
import type { Snapshot, MediaJob } from "../../sdk/media.ts";
import type { SourceState, SourceStamp } from "../../sdk/source.ts";
import { lstat, stat, mkdtemp, rmdir } from "node:fs/promises";
import { join, resolve, dirname, basename, extname } from "node:path";
import { spawn } from "node:child_process";
const VIEW = "capture";
const ENTRIES = [
  ["capture.still", "保存当前画面…", "Screenshot"],
  ["capture.burst", "连拍…", "Burst"],
  ["capture.clip", "导出片段…", "Video clip"],
  ["capture.tasks", "查看导出任务…", "Export tasks"],
] as const;
type Kind = "image" | "burst" | "clip";
type Spec = { kind: Kind; start: number; end?: number; count: number; step: number; processed: boolean; subtitles: boolean; track: string; current: boolean };
type Task = { id: string; spec: Spec; stamp: SourceStamp; source: string; created: string; snapshot?: Snapshot; destination: string; status: string; detail: string; cancel: boolean; jobs: number[]; outputs: { jobId: number; path: string }[]; activeJob?: number; retainUntil?: number; retentionTimer?: ReturnType<typeof setTimeout>; directory?: string; facts: string[] };
export const sameStamp = (a: SourceStamp, b: SourceStamp) => a.generation === b.generation && a.intentVersion === b.intentVersion;
export function parseRange(start: unknown, end?: unknown) {
  const a = Number(String(start).trim());
  const b = end === undefined ? undefined : Number(String(end).trim());
  if (String(start).trim() === "" || !Number.isFinite(a) || a < 0 || (b !== undefined && (!Number.isFinite(b) || b <= a))) throw new Error("Enter valid seconds; end must be after start.");
  return { start: a, end: b };
}
export function sampleTimes(start: number, count: number, step: number, duration: number) {
  if (!Number.isInteger(count) || count < 2 || count > 20 || !Number.isFinite(step) || step < .1 || step > 10 || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(start) || start < 0) throw new Error("Burst requires 2–20 images, 0.1–10s spacing and a finite source duration.");
  const result = Array.from({ length: count }, (_, index) => start + index * step);
  if (result[result.length - 1] >= duration) throw new Error("Burst exceeds source duration. Reduce count or spacing.");
  return result;
}
export async function uniqueFile(path: string) {
  const absolute = resolve(path), extension = extname(absolute), stem = basename(absolute, extension);
  for (let i = 0; i < 1000; i++) {
    const candidate = i ? join(dirname(absolute), `${stem} (${i})${extension}`) : absolute;
    try { await lstat(candidate); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return candidate; throw error; }
  }
  throw new Error("No available filename among the next 1000 suffixes.");
}
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const terminal = (task: Task) => ["completed", "failed", "cancelled"].includes(task.status);
const usable = (source: SourceState) => source.kind === "media" && ["ready", "ended"].includes(source.phase) && source.capabilities.snapshotSource;
const nap = (ms: number) => new Promise<void>(done => setTimeout(done, ms));
export async function activate(host: Host) {
  let disposed = false, opened = false, formBusy = false, operationSequence = 0, taskSequence = 0, runner = false;
  let form: { stamp: SourceStamp; cancel: boolean; guardCurrent: boolean } | null = null;
  let confirmation: ((value: boolean) => void) | null = null;
  let openPromise: Promise<void> | null = null;
  let errorOpened = false;
  const tasks: Task[] = [];
  const nextOperation = () => ++operationSequence;
  const set = async (property: string, value: Json) => { if (opened && !disposed) await host.ui.set(VIEW, property, value); };
  const status = (value: string) => set("status", value);
  const activeTasks = () => tasks.filter(task => !terminal(task));
  async function render() {
    await set("form-busy", formBusy);
    await set("queue-count", `${activeTasks().length}/5 accepted · one running, up to four queued`);
    await set("task-rows", tasks.slice().reverse().map(task => ({
      id: task.id,
      summary: `#${task.id} · ${task.source}\n${task.created} · ${task.spec.kind} · ${task.spec.processed ? "processed" : "raw"} · ${task.status}\n${task.spec.start.toFixed(3)}${task.spec.end === undefined ? "" : `–${task.spec.end}`}s · ${task.outputs.length}/${task.spec.count} published\n${task.detail}\n${task.destination}${terminal(task) && task.status !== "completed" ? task.retainUntil ? `\nRetry input expires ${new Date(task.retainUntil).toLocaleTimeString()}` : "\nInput released; retry requires current source." : ""}${task.facts.length ? `\n${task.facts.at(-1)}` : ""}`,
      cancellable: !terminal(task), retryable: terminal(task) && task.status !== "completed",
      retained: !!task.snapshot && !!task.retainUntil && task.retainUntil > Date.now(), openable: task.outputs.length > 0,
    })));
  }
  async function release(task: Task) {
    if (task.retentionTimer) clearTimeout(task.retentionTimer);
    task.retentionTimer = undefined;
    const snapshot = task.snapshot; task.snapshot = undefined; task.retainUntil = undefined;
    if (snapshot) await host.snapshot.release(snapshot.snapshotId, nextOperation()).catch(() => {});
  }
  async function retain(task: Task) {
    if (!task.snapshot) return;
    task.retainUntil = Date.now() + 300000;
    task.retentionTimer = setTimeout(() => { void release(task).then(render).catch(() => {}); }, 300000);
    task.retentionTimer.unref?.();
  }
  async function trim() {
    while (tasks.length > 20) {
      const index = tasks.findIndex(terminal);
      if (index < 0) break;
      const [old] = tasks.splice(index, 1); await release(old);
    }
  }
  async function mutation<T>(operationId: number, write: () => Promise<T>, recover: (result: unknown) => Promise<T>, report: (value: string) => Promise<void>): Promise<T> {
    try { return await write(); }
    catch (original) {
      while (!disposed) {
        const operation = await host.jobs.operation(operationId).catch(() => null);
        if (operation?.status === "completed") return recover(operation.result);
        if (operation?.status === "failed") throw new Error(`${operation.error.code}: ${operation.error.message}`);
        const code = original && typeof original === "object" && "code" in original ? String(original.code) : "";
        if (operation?.status === "unknown" && /invalid|unsupported|permission|conflict|stale|unavailable/.test(code)) throw original;
        await report(`Operation ${operationId}: outcome unknown; querying this exact ID. ${message(original)}`);
        await nap(1000);
      }
      throw new Error("Plugin stopped with an unknown operation outcome.");
    }
  }
  async function refresh() {
    const source = await host.source.state();
    await set("source-label", usable(source) ? `${source.source} · generation ${source.stamp.generation}` : "Open a finite, independently readable video source.");
    await set("tracks", source.audioTracks?.map(track => `${track.trackId}: ${track.title ?? track.language ?? track.codec ?? "audio"}${track.selected ? " (selected)" : ""}`).join("; ") || "No audio tracks");
    if (form?.guardCurrent && !sameStamp(form.stamp, source.stamp)) {
      form.cancel = true; confirmation?.(false);
      await status("Source or intent changed while preparing. Recheck the new source; existing queued exports retain their original input.");
    }
  }
  async function ensureForm(stamp: SourceStamp) {
    const source = await host.source.state();
    if (disposed || !form || form.cancel || !sameStamp(stamp, source.stamp)) throw new Error("Capture cancelled or source intent changed. Recheck the source.");
    return source;
  }
  const callbacks = ["mark-start", "mark-end", "export-image", "export-burst", "export-clip", "cancel-form", "confirm-start", "reject-start", "cancel-task", "query-task", "open-output", "retry-retained", "retry-current"];
  async function open(section = "Screenshot") {
    if (!opened) {
      if (!openPromise) openPromise = (async () => {
        const current = await host.source.state();
        await host.ui.open(VIEW, "capture.slint", { "start-time": String(current.position ?? 0), "end-time": String(Math.min((current.position ?? 0) + 10, current.duration ?? Infinity)) }, callbacks);
        opened = true;
      })().finally(() => { openPromise = null; });
      await openPromise;
    } else await host.ui.show(VIEW, true);
    await set("section", section); await refresh(); await render();
    if (errorOpened) { await host.ui.close("capture-error"); errorOpened = false; }
  }
  async function reportOpenError(error: unknown) {
    const detail = `Cannot open Capture and export: ${message(error)}. Close this message and choose the capture menu again to retry.`;
    console.error(`[capture] ${detail}`);
    try {
      if (errorOpened) {
        await host.ui.set("capture-error", "message", detail);
        await host.ui.show("capture-error", true);
      } else {
        await host.ui.open("capture-error", "capture-error.slint", { message: detail }, []);
        errorOpened = true;
      }
    } catch (feedbackError) {
      console.error(`[capture] Error window also failed: ${message(feedbackError)}`);
      if (opened) await status(detail).catch(() => {});
    }
  }
  function parseSpec(kind: Kind, args: Json[]): Spec {
    const range = parseRange(args[0], kind === "clip" ? args[1] : undefined);
    return { kind, start: range.start, end: range.end, count: kind === "burst" ? Number(args[1]) : 1, step: kind === "burst" ? Number(args[2]) : 1,
      processed: args[kind === "burst" ? 3 : kind === "clip" ? 2 : 1] === true,
      subtitles: args[kind === "burst" ? 4 : kind === "clip" ? 4 : 2] === true,
      track: kind === "clip" ? String(args[3] ?? "none").trim() : "none",
      current: kind !== "clip" && args[kind === "burst" ? 5 : 3] === true };
  }
  function validate(spec: Spec, source: SourceState) {
    if (!usable(source)) throw new Error("This source cannot be captured. Open a finite video file.");
    if (spec.kind !== "image" && (source.duration === null || !Number.isFinite(source.duration))) throw new Error("Burst and clip export require known finite duration.");
    const start = spec.current ? source.position ?? 0 : spec.start;
    if (spec.kind === "burst") sampleTimes(start, spec.count, spec.step, source.duration!);
    if (source.duration !== null && (start >= source.duration || (spec.end !== undefined && spec.end > source.duration))) throw new Error("The selected range exceeds the source duration.");
  }
  function chooseAudio(spec: Spec, source: SourceState): { kind: "none" } | { kind: "track"; trackId: number } {
    if (spec.kind !== "clip" || spec.track === "none") return { kind: "none" };
    const track = spec.track === "selected" ? source.audioTracks?.find(track => track.selected) : source.audioTracks?.find(track => track.trackId === Number(spec.track));
    if (!track && spec.track === "selected" && !source.audioTracks?.length) return { kind: "none" };
    if (!track || track.external || track.streamIndex === null) throw new Error("Choose a source internal audio track ID or none.");
    return { kind: "track", trackId: track.trackId };
  }
  async function pickDestination(spec: Spec, previous?: string) {
    const chosen = await host.dialogs.open({ kind: spec.kind === "burst" ? "openDirectory" : "saveFile", title: spec.kind === "burst" ? "Choose parent for a unique burst folder" : "Choose export destination", ...(spec.kind === "burst" ? {} : { fileName: previous ? basename(previous) : spec.kind === "clip" ? "capture.mp4" : "capture.png" }) });
    if (chosen.cancelled || !chosen.paths[0]) return null;
    const path = resolve(chosen.paths[0]);
    if (spec.kind === "burst") { if (!(await stat(path)).isDirectory()) throw new Error("Choose an existing directory."); return path; }
    const extension = spec.kind === "clip" ? ".mp4" : ".png";
    if (!path.toLowerCase().endsWith(extension)) throw new Error(`Choose a ${extension} filename.`);
    const available = await uniqueFile(path);
    if (available !== path) await status(`Existing files are protected. Suggested unused filename: ${available}`);
    return available;
  }
  async function confirm(text: string) {
    const answer = new Promise<boolean>(done => { confirmation = done; });
    await set("confirmation", text); await set("confirming", true);
    const accepted = await answer;
    confirmation = null; await set("confirming", false); return accepted;
  }
  async function freeze(spec: Spec, source: SourceState, audio: ReturnType<typeof chooseAudio>) {
    const operationId = nextOperation();
    const time = spec.current ? {} : { time: spec.start };
    let snapshot: Snapshot;
    const recover = async (result: unknown) => {
      if (!result || typeof result !== "object" || !("snapshotId" in result) || !Number.isSafeInteger(result.snapshotId)) throw new Error("Invalid capture operation result.");
      return host.snapshot.state(Number(result.snapshotId));
    };
    if (spec.processed) {
      const [processing, subtitle] = await Promise.all([host.processing.state(), spec.subtitles ? host.subtitles.state() : Promise.resolve(null)]);
      await ensureForm(source.stamp);
      if (processing.source.generation !== source.stamp.generation || (subtitle && subtitle.source.generation !== source.stamp.generation)) throw new Error("Processing or subtitle state belongs to another source.");
      snapshot = await mutation(operationId, () => host.snapshot.capture({ operationId, expected: source.stamp, ...time, mode: "processed", audio, expectedProcessingRevision: processing.configurationRevision, ...(subtitle ? { includeSubtitles: true as const, expectedSubtitle: { selection: subtitle.selection.stamp, presentationRevision: subtitle.presentation.revision } } : { includeSubtitles: false as const }) }), recover, status);
    } else snapshot = await mutation(operationId, () => host.snapshot.capture({ operationId, expected: source.stamp, ...time, mode: "raw", audio }), recover, status);
    return snapshot;
  }
  async function submit(kind: Kind, args: Json[], reuse?: Task) {
    if (formBusy || disposed) return;
    formBusy = true; // exclusion before first await, even when no view is open
    let snapshot: Snapshot | undefined, directory: string | undefined;
    try {
      if (activeTasks().length >= 5) throw new Error("One running and four queued exports are already accepted. Wait or cancel a queued task.");
      const spec = reuse ? { ...reuse.spec, current: false } : parseSpec(kind, args);
      const source = reuse ? null : await host.source.state();
      if (source) validate(spec, source);
      form = { stamp: { ...(source?.stamp ?? reuse!.stamp) }, cancel: false, guardCurrent: !!source };
      await render();
      const audio = source ? chooseAudio(spec, source) : null;
      const chosen = await pickDestination(spec, reuse?.destination);
      if (!chosen) { await status("Save cancelled. No new export was accepted."); return; }
      if (source) await ensureForm(source.stamp);
      if (reuse && (!reuse.snapshot || !reuse.retainUntil || reuse.retainUntil <= Date.now())) throw new Error("Retained snapshot expired. Choose retry from current source.");
      const preview = spec.kind === "burst" ? `\nSamples: ${sampleTimes(spec.current ? source?.position ?? spec.start : spec.start, spec.count, spec.step, source?.duration ?? reuse!.snapshot!.duration).map(time => time.toFixed(3)).join(", ")}s` : "";
      const accepted = await confirm(`Source: ${source?.source ?? reuse!.source}\nGeneration ${form.stamp.generation}; intent ${form.stamp.intentVersion}\n${spec.current ? "Current time will be fixed atomically when capture is accepted" : `Time ${spec.start}${spec.end === undefined ? "" : `–${spec.end}`}s`}${preview}\n${spec.processed ? "Processed" : "Raw: no effects/subtitles"}; subtitles ${spec.processed && spec.subtitles ? "on" : "off"}; audio ${audio?.kind === "track" ? audio.trackId : reuse?.snapshot?.audio?.trackId ?? "none"}\n${chosen}${spec.kind === "burst" ? "/Capture-<unique>/" : ""}\nNo overwrite. ${reuse ? "Reuse immutable retained input." : "Freeze current input at acceptance."}`);
      if (!accepted || form.cancel) { await status("Start cancelled."); return; }
      if (source) { await ensureForm(source.stamp); snapshot = await freeze(spec, source, audio!); }
      else {
        if (!reuse?.snapshot || !reuse.retainUntil || reuse.retainUntil <= Date.now()) throw new Error("Retained snapshot expired during confirmation.");
        snapshot = reuse.snapshot;
        if (reuse.retentionTimer) clearTimeout(reuse.retentionTimer);
        reuse.snapshot = undefined; reuse.retainUntil = undefined; reuse.retentionTimer = undefined;
        snapshot = await host.snapshot.state(snapshot.snapshotId);
      }
      if (source && !sameStamp(snapshot.sourceStamp, source.stamp)) throw new Error("Snapshot source identity mismatch.");
      if (form.cancel || disposed) throw new Error("Capture was cancelled while input was being fixed.");
      spec.start = snapshot.time; spec.current = false;
      if (spec.kind === "burst") sampleTimes(snapshot.time, spec.count, spec.step, snapshot.duration);
      if (spec.kind === "burst") directory = await mkdtemp(join(chosen, "Capture-"));
      const task: Task = { id: String(++taskSequence), spec, stamp: { ...snapshot.sourceStamp }, source: snapshot.source, created: new Date().toISOString(), snapshot, destination: directory ?? chosen, directory, status: "queued", detail: `Fixed snapshot ${snapshot.snapshotId} at ${snapshot.time.toFixed(3)}s${snapshot.kind === "processed" ? `; processing revision ${snapshot.processing.revision}` : ""}`, cancel: false, jobs: [], outputs: [], facts: [] };
      if (reuse) {
        if (reuse.retentionTimer) clearTimeout(reuse.retentionTimer);
        reuse.snapshot = undefined; reuse.retainUntil = undefined; reuse.retentionTimer = undefined;
      }
      snapshot = undefined; directory = undefined; tasks.push(task);
      await trim(); await render(); await status(`Accepted #${task.id}: ${task.source} at ${task.spec.start.toFixed(3)}s. Input remains frozen while queued.`);
      void pump();
    } catch (error) { await status(`Export not accepted: ${message(error)}`); }
    finally {
      if (snapshot && (!reuse || snapshot.snapshotId !== reuse.snapshot?.snapshotId)) await host.snapshot.release(snapshot.snapshotId, nextOperation()).catch(() => {});
      if (directory) await rmdir(directory).catch(() => {});
      form = null; formBusy = false; confirmation = null; await set("confirming", false); await render();
    }
  }
  async function runJob(task: Task, index: number): Promise<MediaJob> {
    if (!task.snapshot) throw new Error("Snapshot is no longer retained.");
    const snapshotId = task.snapshot.snapshotId;
    const sampleTime = task.spec.start + index * task.spec.step;
    const destination = task.spec.kind === "burst" ? join(task.destination, `${String(index + 1).padStart(3, "0")}-${sampleTime.toFixed(3)}s.png`) : task.destination;
    const operationId = nextOperation();
    const report = async (value: string) => { task.status = "unknown"; task.detail = value; await render(); };
    const accepted = await mutation(operationId, () => host.jobs.start({ operationId, snapshotId, destination, ...(task.spec.kind === "clip" ? { kind: task.spec.processed ? "processedMp4" as const : "rawMp4" as const, end: task.spec.end! } : { kind: task.spec.processed ? "processedPng" as const : "rawPng" as const, sampleTime }) }), async result => {
      if (!result || typeof result !== "object" || !("jobId" in result) || !Number.isSafeInteger(result.jobId) || !("snapshotId" in result) || result.snapshotId !== snapshotId) throw new Error("Invalid job-start operation result.");
      return { jobId: Number(result.jobId), status: "queued" as const, snapshotId };
    }, report);
    task.activeJob = accepted.jobId; task.jobs.push(accepted.jobId);
    if (task.cancel || disposed) await host.jobs.cancel(accepted.jobId, nextOperation()).catch(() => {});
    while (!disposed) {
      let job: MediaJob;
      try { job = await host.jobs.state(accepted.jobId); }
      catch (error) { await report(`Job ${accepted.jobId} status unknown: ${message(error)}. Querying, not resubmitting.`); await nap(1000); continue; }
      task.status = ["completed", "failed", "cancelled"].includes(job.status) ? "running" : job.status;
      task.detail = `${task.spec.kind === "burst" ? `Image ${index + 1}/${task.spec.count} at ${sampleTime.toFixed(3)}s · ` : ""}${job.status}${job.publicationStarted ? " · publication started" : ""}`;
      await render();
      if (["completed", "failed", "cancelled"].includes(job.status)) { task.activeJob = undefined; return job; }
      await nap(350);
    }
    throw new Error("Plugin stopped; host owner cleanup cancels unfinished jobs.");
  }
  async function pump() {
    if (runner || disposed) return;
    runner = true;
    try {
      while (!disposed) {
        const task = tasks.find(task => task.status === "queued");
        if (!task) break;
        task.status = "preparing"; await render();
        try {
          for (let index = 0; index < task.spec.count; index++) {
            if (task.cancel || disposed) break;
            const job = await runJob(task, index);
            if (job.facts) task.facts.push(`Video stream ${job.facts.videoStreamIndex}; audio ${job.facts.hasAudio}; ${job.facts.audioTiming}; preroll ${job.facts.preroll ?? "none"}; descriptor ${job.facts.descriptorVersion}; FFmpeg ${job.facts.ffmpegVersion}; range ${JSON.stringify(job.facts.range)}; ${task.snapshot?.kind === "processed" ? `${task.snapshot.presentation.width}×${task.snapshot.presentation.height}` : "source display dimensions"}; SDR 8-bit RGB PNG / H.264+AAC MP4`);
            if (job.status === "completed" && job.outcome && "destination" in job.outcome) task.outputs.push({ jobId: job.jobId, path: job.outcome.destination });
            else if (job.status === "failed") throw new Error(job.outcome && "error" in job.outcome ? `${job.outcome.error.code}: ${job.outcome.error.message}` : "Worker failed without an error outcome.");
            else if (job.status === "cancelled") { task.cancel = true; break; }
            else throw new Error("Worker completed without a confirmed output destination.");
          }
          task.status = task.outputs.length === task.spec.count ? "completed" : "cancelled";
          task.detail = `${task.outputs.length}/${task.spec.count} confirmed published outputs. ${task.outputs.map(output => output.path).join("; ")}`;
        } catch (error) { task.status = "failed"; task.detail = message(error); }
        if (task.status === "completed") await release(task); else await retain(task);
        if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
        await status(`#${task.id} · ${task.source}: ${task.status}. ${task.detail}`); await render();
      }
    } finally { runner = false; }
  }
  async function cancelTask(task: Task) {
    if (terminal(task)) return;
    task.cancel = true;
    if (task.activeJob !== undefined) {
      const result = await host.jobs.cancel(task.activeJob, nextOperation()).catch(error => { task.detail = `Cancellation result unknown: ${message(error)}; querying job until terminal.`; return null; });
      task.status = "cancelling";
      if (result) task.detail = result.cancel === "tooLate" ? "Publication already started; cancellation may be too late. Waiting for actual outcome." : "Cancellation requested; waiting for backend terminal state.";
    } else if (task.status === "queued") {
      task.status = "cancelled"; task.detail = "Cancelled before worker start."; await retain(task);
      if (task.directory) await rmdir(task.directory).catch(() => {});
    } else task.detail = "Cancellation requested while preparing; waiting for job identity/outcome.";
    await render();
  }
  async function openOutput(task: Task) {
    if (!task.outputs.length) throw new Error("No confirmed output exists for this task.");
    const output = task.outputs.at(-1)!;
    const job = await host.jobs.state(output.jobId);
    if (job.status !== "completed" || !job.outcome || !("destination" in job.outcome) || resolve(job.outcome.destination) !== resolve(output.path) || !(await stat(output.path)).isFile()) throw new Error("Published output is missing or no longer confirmed by its original job.");
    const path = resolve(task.directory ?? output.path);
    const program = process.platform === "darwin" ? "open" : process.platform === "win32" ? "explorer.exe" : "xdg-open";
    const args = process.platform === "darwin" ? ["--", path] : [path];
    await new Promise<void>((done, reject) => {
      const child = spawn(program, args, { shell: false, stdio: "ignore" });
      child.once("error", reject); child.once("exit", code => code === 0 ? done() : reject(new Error(`System opener exited ${code}`)));
    });
  }
  const offUi = host.ui.onCallback(event => {
    if (disposed || event.view !== VIEW) return;
    void (async () => {
      const task = tasks.find(task => task.id === String(event.args[0]));
      if (event.callback === "confirm-start") confirmation?.(true);
      else if (event.callback === "reject-start") confirmation?.(false);
      else if (event.callback === "cancel-form") { if (form) form.cancel = true; confirmation?.(false); }
      else if (event.callback === "export-image") await submit("image", event.args);
      else if (event.callback === "export-burst") await submit("burst", event.args);
      else if (event.callback === "export-clip") await submit("clip", event.args);
      else if (event.callback === "mark-start" || event.callback === "mark-end") {
        if (formBusy) return;
        const source = await host.source.state(); await set(event.callback === "mark-start" ? "start-time" : "end-time", String(source.position ?? 0));
      } else if (task) {
        if (event.callback === "cancel-task") await cancelTask(task);
        else if (event.callback === "query-task") {
          const observed = await Promise.all(task.jobs.map(async id => { const job = await host.jobs.state(id); return `Worker ${id}: ${job.status}${job.publicationStarted ? " (publication started)" : ""}`; }));
          await render(); await status(`#${task.id} · ${task.source}: ${task.detail}\n${observed.join("; ")}\n${task.facts.join("\n")}\n${task.outputs.map(output => output.path).join("\n")}`);
        }
        else if (event.callback === "open-output") await openOutput(task);
        else if (event.callback === "retry-retained") {
          if (!terminal(task) || task.status === "completed") throw new Error("Only a failed/cancelled task can be retried.");
          await submit(task.spec.kind, [], task);
        } else if (event.callback === "retry-current") {
          const spec = task.spec;
          const args: Json[] = spec.kind === "clip" ? [String(spec.start), String(spec.end), spec.processed, spec.track, spec.subtitles] : spec.kind === "burst" ? [String(spec.start), String(spec.count), String(spec.step), spec.processed, spec.subtitles, false] : [String(spec.start), spec.processed, spec.subtitles, false];
          await submit(spec.kind, args);
        }
      }
    })().catch(error => { void status(message(error)); });
  });
  const offCommand = host.commands.onExecute(event => {
    const entry = ENTRIES.find(entry => entry[0] === event.id);
    if (entry && event.revision === 1) void open(entry[2]).catch(error => { void reportOpenError(error); });
  });
  for (const [id, title] of ENTRIES) await host.commands.register(id, 1, { title, path: ["视频", "截图与导出", title], shortcut: null }, { enabled: true, checked: null, busy: false, reason: null });
  // Source events arrive ~10/s during playback; keep one refresh in flight and coalesce the rest.
  let refreshing = false, refreshAgain = false;
  const scheduleRefresh = () => {
    if (refreshing) { refreshAgain = true; return; }
    refreshing = true;
    void refresh().catch(error => { void status(message(error)); }).finally(() => {
      refreshing = false;
      if (refreshAgain && !disposed) { refreshAgain = false; scheduleRefresh(); }
    });
  };
  const offSource = await host.events.subscribe("source.state", scheduleRefresh);
  return async () => {
    disposed = true; if (form) form.cancel = true; confirmation?.(false);
    offUi(); offCommand(); await offSource();
    for (const task of tasks) {
      task.cancel = true;
      if (task.activeJob !== undefined) await host.jobs.cancel(task.activeJob, nextOperation()).catch(() => {});
      await release(task);
      if (task.directory && !task.outputs.length) await rmdir(task.directory).catch(() => {});
    }
    if (opened) await host.ui.close(VIEW);
    if (errorOpened) await host.ui.close("capture-error");
  };
}
