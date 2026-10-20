import { mkdtemp, rmdir, stat } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import type { Json, PlayerState, Scope, Snapshot } from "../../sdk/plugin/index.ts";
import { chooseAudio, message, parseSpec, sampleTimes, uniqueFile, validate } from "./model.ts";
import type { Kind, Spec } from "./model.ts";
import type { Task, Tasks } from "./tasks.ts";

/** What the window shows for the form; `main.ts` binds it. */
export interface FormView {
  status: string;
  formBusy: boolean;
  confirmation: string;
  confirming: boolean;
}

/**
 * Prepare → choose destination → confirm → freeze. The snapshot is bound to
 * the source the user saw when starting (`ifCurrent`), so a source change
 * while the save panel or confirmation is open fails instead of capturing
 * other media. A retained-input retry reuses the old snapshot and its source.
 */
export function captureForm(plugin: Scope, view: FormView, tasks: Tasks) {
  const { player } = plugin;
  let form: { seen: PlayerState | null; cancel: boolean } | null = null;
  let answer: ((accepted: boolean) => void) | null = null;
  let stopped = false;
  const say = (text: string) => {
    view.status = text;
    plugin.update();
  };

  player.onChange(() => {
    if (form?.seen && !player.isCurrent(form.seen)) {
      form.cancel = true;
      answer?.(false);
      say("Source or intent changed while preparing. Recheck the new source; existing queued exports retain their original input.");
    }
  });

  async function ensureForm(seen: PlayerState) {
    await player.refresh();
    if (stopped || !form || form.cancel || !player.isCurrent(seen)) throw new Error("Capture cancelled or source intent changed. Recheck the source.");
  }

  async function pickDestination(spec: Spec, previous?: string) {
    const chosen =
      spec.kind === "burst"
        ? await plugin.dialogs.openDirectory({ title: "Choose parent for a unique burst folder" })
        : await plugin.dialogs.saveFile({ title: "Choose export destination", fileName: previous ? basename(previous) : spec.kind === "clip" ? "capture.mp4" : "capture.png" });
    if (!chosen) return null;
    const path = resolve(chosen);
    if (spec.kind === "burst") {
      if (!(await stat(path)).isDirectory()) throw new Error("Choose an existing directory.");
      return path;
    }
    const extension = spec.kind === "clip" ? ".mp4" : ".png";
    if (!path.toLowerCase().endsWith(extension)) throw new Error(`Choose a ${extension} filename.`);
    const available = await uniqueFile(path);
    if (available !== path) say(`Existing files are protected. Suggested unused filename: ${available}`);
    return available;
  }

  async function confirm(text: string) {
    const decided = new Promise<boolean>((done) => (answer = done));
    view.confirmation = text;
    view.confirming = true;
    plugin.update();
    const accepted = await decided;
    answer = null;
    view.confirming = false;
    plugin.update();
    return accepted;
  }

  async function submit(kind: Kind, args: Json[], reuse?: Task) {
    if (view.formBusy || stopped) return;
    view.formBusy = true; // exclusion before the first await
    let snapshot: Snapshot | undefined;
    let directory: string | undefined;
    try {
      if (tasks.active().length >= 5) throw new Error("One running and four queued exports are already accepted. Wait or cancel a queued task.");
      const spec = reuse ? { ...reuse.spec, current: false } : parseSpec(kind, args);
      const seen = reuse ? null : await player.refresh();
      if (seen) validate(spec, seen);
      form = { seen, cancel: false };
      plugin.update();
      const audio = seen ? chooseAudio(spec, seen) : null;
      const chosen = await pickDestination(spec, reuse?.destination);
      if (!chosen) return say("Save cancelled. No new export was accepted.");
      if (seen) await ensureForm(seen);
      if (reuse && !tasks.retained(reuse)) throw new Error("Retained snapshot expired. Choose retry from current source.");
      const duration = seen?.duration ?? reuse?.snapshot?.duration ?? 0;
      const preview = spec.kind === "burst" ? `\nSamples: ${sampleTimes(spec.current ? (seen?.position ?? spec.start) : spec.start, spec.count, spec.step, duration).map((time) => time.toFixed(3)).join(", ")}s` : "";
      const accepted = await confirm(
        `Source: ${seen?.source ?? reuse!.source}\nGeneration ${seen?.sourceId ?? reuse!.sourceId}\n${spec.current ? "Current time will be fixed atomically when capture is accepted" : `Time ${spec.start}${spec.end === undefined ? "" : `–${spec.end}`}s`}${preview}\n${spec.processed ? "Processed" : "Raw: no effects/subtitles"}; subtitles ${spec.processed && spec.subtitles ? "on" : "off"}; audio ${typeof audio === "number" ? audio : (reuse?.snapshot?.audioTrack ?? "none")}\n${chosen}${spec.kind === "burst" ? "/Capture-<unique>/" : ""}\nNo overwrite. ${reuse ? "Reuse immutable retained input." : "Freeze current input at acceptance."}`,
      );
      if (!accepted || form.cancel) return say("Start cancelled.");
      if (seen) {
        await ensureForm(seen);
        // Explicit condition: freeze only the source the user confirmed.
        snapshot = await plugin.media.capture({ ifCurrent: seen, processed: spec.processed, subtitles: spec.processed && spec.subtitles, audio: audio!, ...(spec.current ? {} : { at: spec.start }) });
      } else snapshot = tasks.take(reuse!);
      if (form.cancel || stopped) throw new Error("Capture was cancelled while input was being fixed.");
      spec.start = snapshot.time;
      spec.current = false;
      if (spec.kind === "burst") {
        sampleTimes(snapshot.time, spec.count, spec.step, snapshot.duration);
        directory = await mkdtemp(join(chosen, "Capture-"));
      }
      const task = tasks.add({
        spec,
        sourceId: seen?.sourceId ?? reuse!.sourceId,
        source: snapshot.source,
        created: new Date().toISOString(),
        snapshot,
        destination: directory ?? chosen,
        directory,
        detail: `Fixed snapshot at ${snapshot.time.toFixed(3)}s${snapshot.kind === "processed" ? `; processing revision ${snapshot.processingRevision}` : ""}`,
      });
      snapshot = directory = undefined;
      say(`Accepted #${task.id}: ${task.source} at ${task.spec.start.toFixed(3)}s. Input remains frozen while queued.`);
    } catch (error) {
      say(`Export not accepted: ${message(error)}`);
    } finally {
      await snapshot?.dispose();
      if (directory) await rmdir(directory).catch(() => {});
      form = null;
      answer = null;
      view.formBusy = view.confirming = false;
      plugin.update();
    }
  }

  return {
    submit,
    answer: (accepted: boolean) => answer?.(accepted),
    cancel() {
      if (form) form.cancel = true;
      answer?.(false);
    },
    stop() {
      stopped = true;
      if (form) form.cancel = true;
      answer?.(false);
    },
  };
}
