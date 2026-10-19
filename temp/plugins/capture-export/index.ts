// 实验插件：仅验证公共任务与处理快照，最终 API/界面可能相差很大。
import { basename, dirname, extname, join } from "node:path";
import type {
  JobState,
  PluginContext,
  PluginDisposer,
} from "../../sdk/types.ts";
import { captureSnapshot, finiteRange } from "./snapshot.ts";

type OperationKind = "raw" | "processed" | "burst" | "clip";
interface Operation {
  cancelled: boolean;
  job?: string;
  done?: Promise<void>;
}

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  if (!(await ctx.capabilities()).includes("media.tasks"))
    throw new Error("当前宿主缺少 media.tasks 能力");
  const subscriptions: (() => void)[] = [];
  let closed = false;
  let active: Operation | undefined;
  let includeSubtitles = true;
  let burstCount = 5;
  let burstInterval = 0.5;
  let clipDuration = 5;
  let operationId = 0;

  async function publish(status: string): Promise<void> {
    if (closed) return;
    await ctx.ui.patch({
      status,
      busy: active !== undefined,
      "operation-id": operationId,
      "include-subtitles": includeSubtitles,
      "burst-count": burstCount,
      "burst-interval": burstInterval,
      "clip-duration": clipDuration,
    });
  }

  async function runJob(
    operation: Operation,
    job: JobState,
    expectedSeconds: number,
    completed: number,
    count: number,
  ): Promise<boolean> {
    operation.job = job.id;
    await ctx.ui.patch({ "job-id": job.id });
    for (;;) {
      if (operation.cancelled) await ctx.media.cancel(job.id);
      const status = await ctx.media.status(job.id);
      const within = Math.min(
        1,
        (status.progressSeconds ?? 0) / expectedSeconds,
      );
      if (!closed)
        await ctx.ui.patch({
          "job-state": status.state,
          progress:
            (completed + (status.state === "succeeded" ? 1 : within)) / count,
        });
      if (status.state === "failed")
        throw new Error(status.error ?? "宿主媒体任务失败");
      if (status.state === "cancelled") {
        operation.cancelled = true;
        return false;
      }
      if (status.state === "succeeded") return true;
      await new Promise<void>((resolve) => setTimeout(resolve, 40));
    }
  }

  async function perform(
    operation: Operation,
    kind: OperationKind,
  ): Promise<void> {
    let processingSnapshot: string | undefined;
    let completed = 0;
    try {
      const isClip = kind === "clip";
      const extension = isClip ? "mp4" : "png";
      const selected = await ctx.dialogs.saveFile({
        title: isClip ? "导出 MP4 片段" : "保存 PNG 截图（连续截图自动编号）",
        extensions: [extension],
        suggestedName: `vestra-${Date.now()}.${extension}`,
      });
      if (!selected || operation.cancelled) {
        await publish("已取消选择输出文件");
        return;
      }
      if (extname(selected).toLowerCase() !== `.${extension}`)
        throw new Error(`输出文件必须使用 .${extension} 扩展名`);
      const snapshot = captureSnapshot(await ctx.player.snapshot());
      // 对话框结束后冻结来源、位置、参数与组大小；后续媒体切换不改变本次操作。
      const count = kind === "burst" ? burstCount : 1;
      const interval = burstInterval;
      const requestedDuration = clipDuration;
      if (kind !== "raw")
        processingSnapshot = (await ctx.effects.snapshot({ includeSubtitles }))
          .id;
      const check = await ctx.player.snapshot();
      if (check.sessionId !== snapshot.sessionId)
        throw new Error("准备任务时播放来源发生变化，请重新执行");
      await ctx.ui.patch({
        source: snapshot.input,
        "source-session": snapshot.sessionId,
        progress: 0,
        "completed-count": 0,
        "output-path": selected,
      });
      for (let index = 0; index < count && !operation.cancelled; index++) {
        const position =
          snapshot.position + (kind === "burst" ? index * interval : 0);
        if (snapshot.duration !== null && position >= snapshot.duration)
          throw new Error("连续截图超出媒体时长；已完成的截图保留");
        const duration = isClip
          ? Math.min(
              requestedDuration,
              snapshot.duration === null
                ? requestedDuration
                : snapshot.duration - position,
            )
          : 1;
        const output =
          kind === "burst"
            ? join(
                dirname(selected),
                `${basename(selected, extname(selected))}-${String(index + 1).padStart(3, "0")}.png`,
              )
            : selected;
        const job = await ctx.media.start({
          input: snapshot.input,
          output,
          start: position,
          format: isClip ? "clip" : "image",
          ...(isClip ? { duration } : {}),
          ...(processingSnapshot ? { processingSnapshot } : {}),
        });
        if (!(await runJob(operation, job, duration, completed, count))) break;
        completed++;
        operation.job = undefined;
        await ctx.ui.patch({
          "completed-count": completed,
          "output-path": output,
        });
      }
      await publish(
        operation.cancelled
          ? `已取消，保留 ${completed} 个已完成文件`
          : `已完成 ${completed} 个文件`,
      );
    } catch (error) {
      if (operation.job)
        await ctx.media.cancel(operation.job).catch(console.error);
      await publish(`导出失败：${String(error)}`);
    } finally {
      try {
        if (processingSnapshot)
          await ctx.effects.releaseSnapshot(processingSnapshot);
      } finally {
        if (active === operation) active = undefined;
        if (!closed) await ctx.ui.patch({ busy: false, "job-id": "" });
      }
    }
  }

  function start(kind: OperationKind): void {
    if (active) throw new Error("已有导出任务，请先等待或取消");
    const operation: Operation = { cancelled: false };
    operationId++;
    active = operation;
    // 不阻塞串行 action 队列，否则取消命令无法在任务完成之前到达。
    operation.done = publish("正在准备导出")
      .then(() => perform(operation, kind))
      .catch((error: unknown) => console.error("截图导出清理失败", error));
  }

  async function cancel(): Promise<void> {
    if (!active) return;
    active.cancelled = true;
    if (active.job) await ctx.media.cancel(active.job);
  }

  function on(
    name: string,
    handler: (value: number) => void | Promise<void>,
  ): void {
    subscriptions.push(ctx.onAction(name, handler));
  }
  on("capture-raw", () => start("raw"));
  on("capture-processed", () => start("processed"));
  on("capture-burst", () => start("burst"));
  on("export-clip", () => start("clip"));
  on("cancel", cancel);
  on("include-subtitles", async (value) => {
    if (value !== 0 && value !== 1) throw new Error("字幕开关必须为 0 或 1");
    includeSubtitles = value === 1;
    await publish("已更新字幕采样选项");
  });
  on("burst-count", async (value) => {
    burstCount = finiteRange(value, 1, 100, "连续截图数量", true);
    await publish("已更新连续截图数量");
  });
  on("burst-interval", async (value) => {
    burstInterval = finiteRange(value, 0.1, 60, "截图间隔");
    await publish("已更新截图间隔");
  });
  on("clip-duration", async (value) => {
    clipDuration = finiteRange(value, 0.1, 600, "片段秒数");
    await publish("已更新片段秒数");
  });
  await ctx.ui.patch({
    progress: 0,
    "completed-count": 0,
    "job-id": "",
    "job-state": "",
    "output-path": "",
    source: "",
    "source-session": "",
  });
  await publish("截图与导出就绪");
  return async () => {
    closed = true;
    for (const unsubscribe of subscriptions) unsubscribe();
    const operation = active;
    await cancel();
    await operation?.done;
  };
}
