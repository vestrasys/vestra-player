import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import type { Plugin, PluginError, Stream, StreamStartError } from "@vestra-player/sdk";
import { NesCore } from "./core.ts";

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
  let status = "请选择 iNES ROM 文件。";
  window.bind(() => ({ status, running: !!stream?.running }));

  plugin.resolvers.add({
    id: "nes",
    extensions: ["nes"],
    async resolve(path, signal) {
      pending = latest = path;
      const core = new NesCore(await readRom(path, signal));
      return {
        kind: "stream",
        width: 256,
        height: 240,
        frame: () => core.frame(),
        started(next) {
          stream = next;
          if (pending === path) pending = undefined;
          const source = plugin.player.state.sourceId; // this ROM's open
          status = `正在运行 ${basename(path)} · 256×240 · 48 kHz 立体声`;
          next.onStop(async (error) => {
            if (stream !== next) return; // not when this plugin stops it
            stream = undefined;
            // The stream itself failed (the SDK closed its source): the player's reason.
            if (error && error.code !== "unavailable") return void (status = stopText(error.message));
            // Replaced or closed (F4). The player retires such a stream's producer, so
            // this may arrive as `unavailable: producer absent`: ask what the source is.
            const state = await plugin.player.refresh().catch(() => plugin.player.state);
            if (stream) return; // a newer ROM started meanwhile
            if (state.phase === "empty") return void (status = "已停止。");
            // Still this ROM's source: the player failed it (its reason is in the state).
            if (state.sourceId === source) return void (status = stopText(state.error ?? error?.message));
            // Replaced by another ROM: its start or failure says what happened.
            if (state.source !== latest) status = "已被其他媒体源替换。";
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
      status = `错误：${oneLine(state.error ?? "无法打开 ROM。")}`;
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
      const path = await plugin.dialogs.openFile({ title: "打开 NES ROM" });
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
    status = "已停止。";
  }

  plugin.drops.provide({
    extensions: ["nes"],
    drop: ({ paths }) => open(paths[0]!),
  });
  window.on("openRom", openRom);
  window.on("stop", stop);
  plugin.onError((error) => (status = failureText(error)));
  plugin.onDispose(() => opening?.abort());
  // 模拟器 ▶ (domain 601–619, round allocation table): open, stop, then the window.
  plugin.commands.add({ id: "nes.open", path: ["模拟器", "打开 NES ROM…"], order: 601, run: openRom });
  plugin.commands.add({ id: "nes.stop", path: ["模拟器", "停止 NES"], order: 602, run: stop });
  plugin.commands.add({ id: "nes.window", path: ["模拟器", "NES 工具…"], order: 603, run: () => window.show() });
}

/** The ROM's bytes; file system failures in the plugin's words. */
async function readRom(path: string, signal: AbortSignal) {
  const tooLarge = () => new Error("ROM 文件超过 16 MiB 上限。");
  try {
    if ((await stat(path)).size > MAX_ROM) throw tooLarge();
    const bytes = await readFile(path, { signal });
    if (bytes.byteLength > MAX_ROM) throw tooLarge();
    return bytes;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") throw new Error("找不到 ROM 文件。");
    if (code === "EACCES" || code === "EPERM") throw new Error("没有权限读取 ROM 文件。");
    if (code === "EISDIR") throw new Error("这是一个文件夹，不是 ROM 文件。");
    throw error; // the size limit, or the open was cancelled (AbortError)
  }
}

/** Status text is one line (UI-TOOL-05). */
const oneLine = (text: string) => {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 160 ? `${flat.slice(0, 159)}…` : flat;
};

/** The running game failed: the player's (or SDK's) own reason. */
function stopText(reason: string | null | undefined) {
  return `模拟器已停止：${oneLine(reason ?? "来源已结束。")}`;
}

/** Start-up failures in Chinese; the SDK words these generically. */
const timeouts: Record<StreamStartError["stage"], string> = {
  prefill: "准备首批音视频缓冲超时。",
  activation: "等待模拟器来源启用超时。",
};

function failureText(error: unknown) {
  const { code } = error as PluginError;
  if (code === "timeout") return `错误：${timeouts[(error as StreamStartError).stage] ?? "启动超时。"}`;
  if (code === "device") return "错误：音频设备未就绪。";
  return `错误：${oneLine(error instanceof Error ? error.message : String(error))}`;
}
