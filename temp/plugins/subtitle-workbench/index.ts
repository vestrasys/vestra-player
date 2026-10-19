// 实验字幕工作台；接口和生命周期策略可能与最终产品差异很大。
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";
import {
  parseSrt,
  serializeSrt,
  subtitleFilter,
  type Cue,
} from "./subtitles.ts";
import {
  MockTranslationProvider,
  type TranslationProvider,
} from "./translation.ts";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  return activateWithProvider(ctx, new MockTranslationProvider());
}
export async function activateWithProvider(
  ctx: PluginContext,
  provider: TranslationProvider,
): Promise<PluginDisposer> {
  if (!(await ctx.capabilities()).includes("effects.subtitle"))
    throw new Error("宿主不支持 effects.subtitle");
  const directory = await mkdtemp(join(tmpdir(), "vestra-subtitles-"));
  const cache = join(directory, "active.srt");
  let session = (await ctx.player.snapshot()).sessionId;
  let original: Cue[] = [];
  let translated: Cue[] = [];
  let translatedSelected = false;
  let visible = true;
  let delay = 0;
  let fontSize = 24;
  let yellow = false;
  let cueIndex = 0;
  let closed = false;
  let revision = 0;
  let active: { controller: AbortController; task: Promise<void> } | undefined;
  const subscriptions: (() => void)[] = [];
  const currentCues = () => (translatedSelected ? translated : original);
  async function publish(status: string): Promise<void> {
    if (!closed)
      await ctx.ui.patch({
        status,
        visible,
        delay,
        "font-size": fontSize,
        yellow,
        "cue-count": currentCues().length,
        "cue-index": cueIndex + 1,
        translated: translatedSelected,
        translating: Boolean(active),
      });
  }
  async function apply(status: string): Promise<void> {
    if (!visible || !currentCues().length) await ctx.effects.remove("captions");
    else {
      if (!session) throw new Error("当前没有可绑定的播放会话");
      await writeFile(cache, serializeSrt(currentCues(), delay), "utf8");
      await ctx.effects.set({
        key: "captions",
        kind: "subtitle",
        sessionId: session,
        filter: subtitleFilter("captions", fontSize, yellow),
        files: { captions: cache },
      });
    }
    await publish(status);
  }
  async function cancel(): Promise<void> {
    revision++;
    const pending = active;
    pending?.controller.abort();
    await pending?.task;
    if (active === pending) active = undefined;
  }
  function on(
    name: string,
    handler: (value: number) => void | Promise<void>,
  ): void {
    subscriptions.push(ctx.onAction(name, handler));
  }
  on("open", async () => {
    const path = await ctx.dialogs.openFile({
      extensions: ["srt"],
      title: "加载 SRT 字幕",
    });
    if (!path) return;
    const next = parseSrt(await readFile(path, "utf8"));
    await cancel();
    const state = await ctx.player.snapshot();
    if (!state.path || !state.sessionId)
      throw new Error("请先打开媒体再加载字幕");
    session = state.sessionId;
    original = next;
    translated = [];
    translatedSelected = false;
    cueIndex = 0;
    await apply(`已加载 ${original.length} 条字幕`);
  });
  on("toggle", async () => {
    visible = !visible;
    await apply(visible ? "字幕已显示" : "字幕已隐藏");
  });
  on("delay", async (value) => {
    serializeSrt(original, value);
    delay = value;
    await apply("字幕同步已更新");
  });
  on("font-size", async (value) => {
    subtitleFilter("captions", value, yellow);
    fontSize = value;
    await apply("字体已更新");
  });
  on("yellow", async () => {
    yellow = !yellow;
    await apply("字幕颜色已更新");
  });
  on("original", async () => {
    translatedSelected = false;
    await apply("已选择原文字幕");
  });
  on("translated", async () => {
    if (!translated.length) throw new Error("请先翻译字幕");
    translatedSelected = true;
    await apply("已选择译文字幕");
  });
  on("next-cue", async () => {
    if (!currentCues().length) throw new Error("尚未加载字幕");
    cueIndex = (cueIndex + 1) % currentCues().length;
    await ctx.player.seek(currentCues()[cueIndex].start + delay);
    await publish(`选中第 ${cueIndex + 1} 条字幕`);
  });
  on("mock-fail-next", async () => {
    if (provider instanceof MockTranslationProvider) provider.failNext = true;
    await publish("下一次 mock 翻译将失败");
  });
  on("cancel", async () => {
    await cancel();
    await publish("翻译已取消，原字幕保留");
  });
  on("translate", async () => {
    if (!original.length || !session) throw new Error("尚未加载字幕");
    await cancel();
    const token = ++revision;
    const sourceSession = session;
    const cues = original.map((cue) => ({ ...cue }));
    const controller = new AbortController();
    // 不等待网络任务，运行时仍能处理取消与播放会话切换。
    const task = (async () => {
      try {
        const result = await provider.translate(cues, controller.signal);
        const snapshot = await ctx.player.snapshot();
        if (
          closed ||
          controller.signal.aborted ||
          revision !== token ||
          snapshot.sessionId !== sourceSession
        )
          return;
        translated = result;
        translatedSelected = true;
        await apply("翻译完成，已选择译文");
      } catch (error) {
        if (!closed && !controller.signal.aborted && revision === token)
          await publish(`翻译失败，可重试：${String(error)}`);
      } finally {
        if (active?.controller === controller) {
          active = undefined;
          if (!closed) await ctx.ui.patch({ translating: false });
        }
      }
    })();
    active = { controller, task };
    await publish("翻译中（确定性 mock 服务）");
  });
  subscriptions.push(
    ctx.onPlayback(async (state) => {
      if (state.sessionId === session) return;
      await cancel();
      session = state.sessionId;
      original = [];
      translated = [];
      translatedSelected = false;
      cueIndex = 0;
      await ctx.effects.remove("captions");
      await publish("媒体已切换，请重新加载字幕");
    }),
  );
  await publish("字幕工作台就绪");
  return async () => {
    closed = true;
    for (const unsubscribe of subscriptions) unsubscribe();
    await cancel();
    try {
      await ctx.effects.remove("captions");
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  };
}
