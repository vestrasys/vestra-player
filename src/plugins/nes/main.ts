import { readFile } from "node:fs/promises";
import { basename } from "node:path";
import type { Plugin, PluginError, Stream, StreamStartError } from "@vestra-player/sdk";
import { NesCore } from "./core.ts";

/** NES emulator: one tool window, one generated AV stream, three menu entries. */
export default async function activate(plugin: Plugin) {
  const window = plugin.windows.create("nes.slint");
  let stream: Stream | undefined;
  let opening: AbortController | undefined;
  let status = "请选择 iNES ROM 文件。";
  window.bind(() => ({ status, running: !!stream?.running }));

  async function openRom() {
    if (opening) return; // one open at a time
    const attempt = (opening = new AbortController());
    const { signal } = attempt;
    try {
      await window.show();
      const path = await plugin.dialogs.openFile({ title: "打开 NES ROM" });
      if (!path || signal.aborted) return;
      const bytes = await readFile(path);
      if (signal.aborted) return;
      if (bytes.byteLength > 16 * 1024 * 1024) throw new Error("ROM 超过 16 MiB 上限。");
      const core = new NesCore(bytes);
      await stream?.stop(); // a bad ROM keeps the running game
      if (signal.aborted) return;
      // The SDK prerolls, activates, acknowledges pause/input controls and paces frame().
      const next = await plugin.player.openStream({ width: 256, height: 240, frame: () => core.frame(), signal });
      stream = next;
      next.onStop((error) => (status = stopText(error)));
      status = `正在运行 ${basename(path)} · 256×240 · 48 kHz 立体声`;
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
    status = "已停止。";
  }

  window.on("openRom", openRom);
  window.on("stop", stop);
  plugin.onError((error) => (status = failureText(error)));
  plugin.onDispose(() => opening?.abort());
  // 模拟器 ▶ (domain 601–619, round allocation table): open, stop, then the window.
  plugin.commands.add({ id: "nes.open", path: ["模拟器", "打开 NES ROM…"], order: 601, run: openRom });
  plugin.commands.add({ id: "nes.stop", path: ["模拟器", "停止 NES"], order: 602, run: stop });
  plugin.commands.add({ id: "nes.window", path: ["模拟器", "NES 工具…"], order: 603, run: () => window.show() });
}

/** Status text is one line (UI-TOOL-05). */
const oneLine = (text: string) => {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 160 ? `${flat.slice(0, 159)}…` : flat;
};

function stopText(error: PluginError | null) {
  if (!error) return "已被其他媒体源替换。";
  return `模拟器已停止：${oneLine(error.message)}`; // the host's own reason
}

/** Start-up failures in Chinese; the SDK words these generically. */
const timeouts: Record<StreamStartError["stage"], string> = {
  prefill: "准备首批音视频缓冲超时。",
  activation: "等待模拟器来源启用超时。",
};

function failureText(error: unknown) {
  const { code, message } = error as PluginError;
  if (code === "timeout") return `错误：${timeouts[(error as StreamStartError).stage] ?? "启动超时。"}`;
  if (code === "device") return "错误：音频设备未就绪。";
  if (code === "stream" && message === "producer epoch changed before activation") return "错误：启动期间来源已变化，请重试。";
  return `错误：${oneLine(error instanceof Error ? error.message : String(error))}`;
}
