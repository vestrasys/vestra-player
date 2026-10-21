import type { PackageCandidate, PackageChange, PackageTrial, Plugin, PluginError } from "@vestra-player/sdk";
import { WorkshopJob, defaultRequirement, requirement } from "./engine.mjs";
import * as fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";

/** Status text is one line (UI-TOOL-05). */
const oneLine = (text: string) => {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 160 ? `${flat.slice(0, 159)}…` : flat;
};

/** Package change states in Chinese, after 「安装」/「回退」. */
const changeText: Record<string, string> = {
  preparing: "：正在准备…",
  activating: "：正在启用…",
  restoring: "：正在恢复原版本…",
  committed: "完成。",
  cancelled: "已取消。",
  failed: "失败。",
  failedDegraded: "失败，未能完全恢复原版本。",
  tooLate: "已无法取消，操作继续。",
};

const tools: Record<string, string> = {
  write_candidate: "写入候选文件",
  read_candidate: "读取候选文件",
  validate_candidate: "真实TypeScript校验",
};

/** One readable log line per engine event, without its internal ids, revisions or digests. */
export function logLine(event: { phase: string; text: string }) {
  let value: Record<string, unknown>;
  try {
    value = JSON.parse(event.text);
  } catch {
    return `${event.phase}：${event.text}`;
  }
  if (event.phase === "真实TypeScript诊断") {
    const output = String(value.stdout ?? "").trim();
    return value.code === 0 ? "真实TypeScript诊断：通过" : `真实TypeScript诊断：未通过\n${output}`;
  }
  const name = tools[String(value.toolName)] ?? String(value.toolName);
  const file = (value.args as { path?: unknown } | undefined)?.path;
  if (value.type === "tool_execution_start") return `Pi工具：${name}${typeof file === "string" ? ` ${file}` : ""}`;
  if (value.type === "tool_execution_end") return `Pi工具：${name}${value.isError ? "失败" : "完成"}`;
  return event.phase;
}

/** Pi plugin workshop: generate a counter plugin, let the host validate it, trial, install, roll back. */
export default async function activate(plugin: Plugin) {
  let busy = false;
  let starting = false; // a trial start is in flight
  let draft = defaultRequirement;
  let job: WorkshopJob | undefined;
  let candidate: PackageCandidate | undefined;
  let trial: PackageTrial | undefined;
  let change: PackageChange | undefined;
  let selectedId = "community.counter";
  let log = "";
  let status = "选择工作区并输入需求后生成。";
  let workspace = "";
  const idle = () => !busy && !trial;

  /** Progress inside long handlers is shown right away, not when the handler ends. */
  const publish = (text: string) => {
    log = (log + "\n" + text).slice(-16000);
    plugin.update();
  };
  const show = (text: string) => {
    status = text;
    plugin.update();
  };
  const fail = (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    show(`错误：${oneLine(message)}`);
    publish(`错误：${message}`);
  };

  const window = plugin.windows.create("workshop.slint");
  window.bind(() => ({
    requirementText: draft,
    logText: log,
    statusText: status,
    workspaceText: workspace || "尚未选择：请选择专用空文件夹",
    busy,
    canTrial: !!candidate && idle(),
    canStop: !!trial,
    canRecover: false, // a lost trial reply is recovered by the SDK
    canInstall: !!candidate && idle(),
  }));

  const generate = async () => {
    if (!idle()) throw Error("请先停止试载并等待当前任务完成");
    if (!workspace) throw Error("请先选择本次工坊专用工作区");
    requirement(draft);
    busy = true;
    candidate = undefined;
    const current = (job = new WorkshopJob(path.join(workspace, `candidate-${randomUUID()}`), (event) =>
      publish(logLine(event)),
    ));
    try {
      show("正在生成候选插件（本地模型，不联网）…");
      const result = await current.run(draft);
      selectedId = current.requirement.id;
      show("类型检查通过，正在由播放器校验…");
      const checked = await plugin.packages.validate(result.directory);
      if (current.cancelled) throw Error("任务已取消");
      candidate = checked;
      publish(`宿主校验通过：${checked.manifest.id} ${checked.manifest.version}（${checked.checks.join("、")}），位于 ${result.directory}`);
      show(`候选已通过宿主校验：${checked.manifest.id} ${checked.manifest.version}，可试载或安装。`);
    } catch (error) {
      candidate = undefined;
      if (current.cancelled) show("生成已取消。");
      else fail(error);
    } finally {
      busy = false;
    }
  };

  const stopTrial = async () => {
    if (!trial) return;
    try {
      await trial.stop(); // waits until the host confirmed cleanup
    } catch (error) {
      // Keep the trial: stopping again re-asks the host.
      if ((error as PluginError).code === "cleanup") throw Error("播放器未确认试载已清理，请再次停止试载。");
      throw error;
    }
    await trial.dispose();
    trial = undefined;
    show("试载已停止并清理。");
  };

  /** Install or roll back; only ever on an explicit click, and never resubmitted. */
  const manage = async (rollback: boolean) => {
    if (!idle()) throw Error("请先等待任务完成并停止试载");
    if (!rollback && !candidate) throw Error("请先生成并通过宿主校验");
    busy = true;
    const action = rollback ? "回退" : "安装";
    publish(`用户明确${action} ${selectedId}`);
    let current: PackageChange;
    try {
      current = rollback ? await plugin.packages.rollback(selectedId) : await candidate!.install();
    } catch (error) {
      busy = false;
      throw error;
    }
    change = current;
    const report = () => {
      const text = `${action}${changeText[current.status] ?? "状态未知。"}`;
      publish(current.message ? `${text}（${current.message}）` : text);
      show(current.message && current.status.startsWith("failed") ? `${text}原因：${oneLine(current.message)}` : text);
    };
    const listening = current.onChange(report);
    report();
    try {
      await current.done;
    } finally {
      await listening.dispose();
      change = undefined;
      busy = false;
    }
  };

  window.on("draftChanged", (text: string) => {
    draft = text;
  });
  window.on("chooseWorkspace", async () => {
    if (!idle()) throw Error("活动任务期间不能更换工作区");
    const chosen = await plugin.dialogs.openDirectory({ title: "选择工坊专用工作区（每个候选写入新子目录）" });
    if (!chosen) return;
    workspace = await fs.realpath(chosen);
    candidate = undefined;
  });
  window.on("chooseRequirement", async () => {
    const chosen = await plugin.dialogs.openFile({ title: "选择计数器需求JSON" });
    if (!chosen) return;
    if ((await fs.stat(chosen)).size > 65536) throw Error("需求超过64KiB");
    const text = await fs.readFile(chosen, "utf8");
    requirement(text);
    draft = text;
    show("需求已读取，点击生成。");
  });
  window.on("generate", generate);
  window.on("cancelTask", async () => {
    if (job && busy && !change && !starting) {
      job.cancel();
      await job.agent?.waitForIdle();
      show("正在取消生成…");
    }
    if (trial) await stopTrial();
    if (change) await change.cancel();
  });
  window.on("startTrial", async () => {
    if (!candidate || !idle()) throw Error("候选尚未就绪");
    starting = busy = true;
    plugin.update();
    try {
      trial = await candidate.trial(); // resolves once the trial has started
    } finally {
      starting = busy = false;
    }
    publish(`试载已开始：${selectedId}`);
    show("试载中：请在计数器窗口测试「增加」「归零」。");
  });
  window.on("recoverTrial", () => {}); // kept for the window; the button stays disabled
  window.on("stopTrial", stopTrial);
  window.on("installCandidate", () => manage(false));
  window.on("rollbackVersion", () => manage(true));
  plugin.onError(fail);
  plugin.onDispose(async () => {
    job?.cancel();
    await job?.agent?.waitForIdle();
  });

  plugin.commands.add({
    id: "workshop.open",
    path: ["插件工坊…"],
    order: 820,
    group: "windows",
    run: () => window.show(),
  });
}
