// 实验 NES 来源插件；最终 API 可能相差很大。ROM 执行/时钟属于插件，宿主管理呈现资源。
import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";
import { Emulator } from "./engine.ts";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  let emulator: Emulator | undefined;
  let rom: Buffer | undefined;
  let romName = "";
  let sessionId: string | undefined;
  let paused = false;
  let disposed = false;
  let frames = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: Promise<void> = Promise.resolve();
  const key = "emulator";
  const update = (status: string) =>
    ctx.ui.patch({ status, loaded: !!emulator, paused, frames });
  const stopTimer = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
  };
  async function tick() {
    const started = performance.now();
    if (!emulator || !sessionId || paused || disposed) return;
    try {
      const output = emulator.frame();
      await ctx.sources.present({
        key,
        sessionId,
        rgb: output.rgb.toString("base64"),
        pcm: output.pcm.toString("base64"),
      });
      frames++;
      if (frames % 30 === 0) await update(`${romName} · 正在运行`);
      if (!paused && !disposed)
        timer = setTimeout(
          () => {
            pending = tick();
          },
          Math.max(0, 1000 / 60 - (performance.now() - started)),
        );
    } catch (error) {
      paused = true;
      await ctx.sources.setPaused({ key, paused: true }).catch(() => undefined);
      await update(
        `呈现失败，已停止：${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  async function quiesce() {
    stopTimer();
    await pending;
    stopTimer();
  }
  async function close() {
    paused = true;
    await quiesce();
    if (sessionId) await ctx.sources.close(key);
    sessionId = undefined;
    emulator = undefined;
    rom = undefined;
    frames = 0;
    await update("NES 会话已关闭");
  }
  const subscriptions: (() => void)[] = [];
  const action = (name: string, handler: () => Promise<void>) =>
    subscriptions.push(
      ctx.onAction(name, async () => {
        try {
          await handler();
        } catch (error) {
          await update(
            `操作失败：${error instanceof Error ? error.message : String(error)}`,
          );
        }
      }),
    );
  action("open-rom", async () => {
    const path = await ctx.dialogs.openFile({
      extensions: ["nes"],
      title: "打开合法 NES ROM",
    });
    if (!path) return;
    const bytes = await readFile(path);
    const candidate = new Emulator(bytes); // Validate before replacing the running source.
    await quiesce();
    let source: { sessionId: string };
    try {
      source = await ctx.sources.open({
        key,
        width: 256,
        height: 240,
        sampleRate: 44100,
      });
    } catch (error) {
      if (emulator && !paused) pending = tick();
      throw error;
    }
    emulator = candidate;
    rom = bytes;
    romName = basename(path);
    sessionId = source.sessionId;
    frames = 0;
    paused = false;
    await update(`${romName} · 正在运行`);
    pending = tick();
  });
  action("toggle-pause", async () => {
    if (!emulator) throw new Error("尚未打开 ROM");
    paused = !paused;
    if (paused) await quiesce();
    await ctx.sources.setPaused({ key, paused });
    if (!paused) pending = tick();
    await update(`${romName} · ${paused ? "已暂停" : "正在运行"}`);
  });
  action("save-state", async () => {
    if (!emulator) throw new Error("尚未打开 ROM");
    const path = await ctx.dialogs.saveFile({
      title: "保存 NES 状态",
      extensions: ["json"],
      suggestedName: `${romName}.state.json`,
    });
    if (!path) return;
    // Capture synchronously at a frame boundary; encoding/file I/O do not alter emulator state.
    const saved = emulator.save();
    await writeFile(path, saved, { flag: "wx" });
    await update(`状态已保存：${basename(path)}`);
  });
  action("load-state", async () => {
    if (!emulator || !rom) throw new Error("尚未打开 ROM");
    const path = await ctx.dialogs.openFile({
      title: "恢复 NES 状态",
      extensions: ["json"],
    });
    if (!path) return;
    const serialized = await readFile(path, "utf8");
    const candidate = new Emulator(rom);
    candidate.restore(serialized); // A failed restore never damages the live core.
    await quiesce();
    emulator = candidate;
    await update(`状态已恢复：${basename(path)}`);
    if (!paused) pending = tick();
  });
  action("close-rom", close);
  subscriptions.push(
    ctx.onInput((event) => emulator?.input(event.key, event.pressed)),
  );
  await update("请选择自备合法 NES ROM");
  return async () => {
    disposed = true;
    for (const unsubscribe of subscriptions) unsubscribe();
    await close();
  };
}
