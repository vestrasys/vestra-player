import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import type { Plugin, PluginError, Stream, StreamStartError, Text, TextError } from "@vestra-player/sdk";
import { type Fail, NesCore } from "./core.ts";

const MAX_ROM = 16 * 1024 * 1024;

/**
 * NES emulator: `.nes` files open like media from any entry (⌘O, drop,
 * playlist) through a file resolver; one tool window, three menu entries.
 */
export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("nes.slint");
  let stream: Stream | undefined;
  /** The ROM being opened (read, then prepared) until its stream starts or fails. */
  let pending: string | undefined;
  /** The ROM this resolver was last asked for: a source change to it is ours to word. */
  let latest: string | undefined;
  /** The menu / button attempt (picker, then its open): Stop aborts it. */
  let opening: AbortController | undefined;
  /** The picker is up: a second one is ignored. */
  let picking = false;
  /** What the window says (worded in nes.slint): a state, the ROM's name, a reason. */
  let status: Status = { state: "idle" };
  window.bind(() => ({ state: status.state, rom: status.rom ?? "", detail: status.detail ?? "", running: !!stream?.running }));

  plugin.resolvers.add({
    id: "nes",
    extensions: ["nes"],
    async resolve(path, signal) {
      pending = latest = path;
      const core = new NesCore(await readRom(path, signal, plugin.i18n.error), plugin.i18n.error);
      return {
        kind: "stream",
        width: 256,
        height: 240,
        frame: () => core.frame(),
        started(next) {
          stream = next;
          if (pending === path) pending = undefined;
          status = { state: "running", rom: basename(path) };
          next.onStop((error) => {
            if (stream !== next) return; // not when this plugin stops it, nor after a newer ROM started
            stream = undefined;
            // The stream failed: the player's reason (or this plugin's, for a frame failure).
            if (error) return void (status = stopText(error));
            // Replaced or closed: the player's state (read by the SDK) says which.
            const state = plugin.player.state;
            if (state.phase === "empty") status = { state: "stopped" };
            // Replaced by another source; a newer ROM words its own start or failure.
            else if (state.source !== latest) status = { state: "replaced" };
          });
        },
      };
    },
  });
  // A ROM that fails to open (bad file, stream start-up): the player shows the
  // failure too; the window says it in one line.
  plugin.player.onChange((state) => {
    if (state.phase === "error" && pending !== undefined && state.source === pending) {
      pending = undefined;
      status = state.error == null ? { state: "openFailed" } : { state: "error", detail: oneLine(state.error) };
    }
  });

  /** `player.open` from this plugin's own entries (picker, drop). */
  async function open(path: string, signal?: AbortSignal) {
    try {
      await plugin.player.open(path, { signal });
    } catch (error) {
      const { code } = error as PluginError;
      // Stopped, or a newer open took over; a load failure is worded from the player state.
      if (signal?.aborted || code === "replaced" || code === "backend") return;
      status = failureText(error);
    }
  }
  async function openRom() {
    if (picking) return; // one picker at a time
    const attempt = (opening = new AbortController());
    picking = true;
    try {
      await window.show();
      const path = await plugin.dialogs.openFile({ title: "Open NES ROM" });
      picking = false;
      if (path && !attempt.signal.aborted) await open(path, attempt.signal);
    } catch (error) {
      if (!attempt.signal.aborted) status = failureText(error); // Stop cancels silently
    } finally {
      picking = false;
      if (opening === attempt) opening = undefined;
    }
  }
  async function stop() {
    opening?.abort();
    const state = plugin.player.state;
    // A ROM still opening (being read or prepared, or selected but not started):
    // close that open, which aborts the resolution.
    if (pending !== undefined && state.source === pending && (state.phase === "loading" || state.phase === "ready"))
      await plugin.player.close({ ifCurrent: state }).catch(() => {});
    pending = undefined;
    const current = stream;
    stream = undefined;
    await current?.stop(); // closes the source only if it is still ours
    status = { state: "stopped" };
  }

  plugin.drops.provide({
    extensions: ["nes"],
    drop: ({ paths }) => open(paths[0]!),
  });
  window.on("openRom", openRom);
  window.on("stop", stop);
  plugin.onError((error) => (status = failureText(error)));
  plugin.onDispose(() => opening?.abort());
  // Emulator ▸ (domain 601–619, round allocation table): open, stop, then the window.
  plugin.commands.add({ id: "nes.open", path: ["Emulator", "Open NES ROM…"], order: 601, run: openRom });
  plugin.commands.add({ id: "nes.stop", path: ["Emulator", "Stop NES"], order: 602, run: stop });
  plugin.commands.add({ id: "nes.window", path: ["Emulator", "NES Tools…"], order: 603, run: () => window.show() });
}

/** The ROM's bytes; file system failures in the plugin's words. */
async function readRom(path: string, signal: AbortSignal, error: Fail) {
  const tooLarge = () => error("The ROM file is larger than the 16 MiB limit.");
  try {
    if ((await stat(path)).size > MAX_ROM) throw tooLarge();
    const bytes = await readFile(path, { signal });
    if (bytes.byteLength > MAX_ROM) throw tooLarge();
    return bytes;
  } catch (failure) {
    const code = (failure as NodeJS.ErrnoException).code;
    if (code === "ENOENT") throw error("The ROM file was not found.");
    if (code === "EACCES" || code === "EPERM") throw error("No permission to read the ROM file.");
    if (code === "EISDIR") throw error("This is a folder, not a ROM file.");
    throw failure; // the size limit, or the open was cancelled (AbortError)
  }
}

/**
 * The window's status line, worded in nes.slint. `detail` is a reason in the
 * player's (or the SDK's) words, shown in one line, or this plugin's own
 * (a text the player translates).
 */
type Status = {
  state:
    | "idle"
    | "running"
    | "stopped"
    | "replaced"
    | "halted"
    | "ended"
    | "error"
    | "openFailed"
    | "timeout"
    | "prefillTimeout"
    | "activationTimeout"
    | "device";
  rom?: string;
  detail?: string | Text;
};

/** Status text is one line (UI-TOOL-05). */
const oneLine = (text: string) => {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 160 ? `${flat.slice(0, 159)}…` : flat;
};

/** Why something failed: this plugin's own words (`plugin.i18n.error`), else the error's message in one line. */
const reasonOf = (error: unknown): string | Text =>
  (error as Partial<TextError>)?.text ?? oneLine(error instanceof Error ? error.message : String(error));

/** The running game failed: the player's (or SDK's, or this plugin's) own reason. */
function stopText(error: PluginError | null): Status {
  return error == null ? { state: "ended" } : { state: "halted", detail: reasonOf(error) };
}

/** Start-up failures the SDK words generically get the plugin's own words. */
const timeouts: Record<StreamStartError["stage"], Status["state"]> = {
  prefill: "prefillTimeout",
  activation: "activationTimeout",
};

function failureText(error: unknown): Status {
  const { code } = error as PluginError;
  if (code === "timeout") return { state: timeouts[(error as StreamStartError).stage] ?? "timeout" };
  if (code === "device") return { state: "device" };
  return { state: "error", detail: reasonOf(error) };
}
