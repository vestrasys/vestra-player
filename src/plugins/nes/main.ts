import { readFile } from "node:fs/promises";
import { basename } from "node:path";
import type { Plugin, PluginError, Stream, StreamStartError } from "../../sdk/plugin/index.ts";
import { NesCore } from "./core.ts";

/** NES emulator: one tool window, one generated AV stream, three menu entries. */
export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("nes.slint");
  let stream: Stream | undefined;
  let opening: AbortController | undefined;
  let status = "Choose an iNES ROM to start.";
  window.bind(() => ({ status, running: !!stream?.running }));

  async function openRom() {
    if (opening) return; // one open at a time
    const attempt = (opening = new AbortController());
    const { signal } = attempt;
    try {
      await window.show();
      const path = await plugin.dialogs.openFile({ title: "Open NES ROM" });
      if (!path || signal.aborted) return;
      const bytes = await readFile(path);
      if (signal.aborted) return;
      if (bytes.byteLength > 16 * 1024 * 1024) throw new Error("ROM exceeds the 16 MiB limit");
      const core = new NesCore(bytes);
      await stream?.stop(); // a bad ROM keeps the running game
      if (signal.aborted) return;
      // The SDK prerolls, activates, acknowledges pause/input controls and paces frame().
      const next = await plugin.player.openStream({ width: 256, height: 240, frame: () => core.frame(), signal });
      stream = next;
      next.onStop((error) => (status = stopText(error)));
      status = `Running ${basename(path)} · 256 × 240 · stereo 48 kHz`;
    } catch (error) {
      if (!signal.aborted) status = failureText(error); // Stop cancels silently
    } finally {
      opening = undefined;
    }
  }
  async function stop() {
    opening?.abort(); // a pending open never selects its stream afterwards
    await stream?.stop(); // closes the source only if it is still ours
    stream = undefined;
    status = "Stopped.";
  }

  window.on("openRom", openRom);
  window.on("stop", stop);
  plugin.onError((error) => (status = String(error)));
  plugin.onDispose(() => opening?.abort());
  plugin.commands.add({ id: "nes.open", path: ["模拟器", "打开 NES ROM…"], order: 563, run: openRom });
  plugin.commands.add({ id: "nes.window", path: ["模拟器", "NES 工具"], order: 561, run: () => window.show() });
  plugin.commands.add({ id: "nes.stop", path: ["模拟器", "停止 NES"], order: 562, run: stop });
}

function stopText(error: PluginError | null) {
  if (!error) return "NES source replaced.";
  if (error.code === "stream") return error.message; // the host's own reason
  return `NES stopped: ${String(error)}`;
}

/** The plugin's original start-up wording; the SDK words these failures generically. */
const timeouts: Record<StreamStartError["stage"], string> = {
  prefill: "Timed out preparing first NES AV buffers",
  activation: "Timed out waiting for NES source activation",
};
const startup: Record<string, string> = {
  "producer epoch changed before activation": "Producer epoch changed before activation",
};

function failureText(error: unknown) {
  const { code, message } = error as PluginError;
  if (code === "timeout") return `Error: ${timeouts[(error as StreamStartError).stage] ?? message}`;
  if (code === "stream" || code === "device")
    return `Error: ${startup[message] ?? message.replace(/^audio device/, "Audio device")}`;
  return String(error);
}
