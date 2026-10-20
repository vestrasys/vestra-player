import type { PackageCandidate, PackageChange, PackageTrial, Plugin } from "../../sdk/plugin/index.ts";
import { WorkshopJob, defaultRequirement, requirement } from "./engine.mjs";
import * as fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";

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
  let status = "选择工作区并输入需求后生成";
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
    show(`错误：${String(error)}`);
    publish(String(error));
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
      publish(`${event.phase}: ${event.text}`),
    ));
    try {
      show("Pi Agent运行中 · 模型本地mock · 工具真实执行");
      const result = await current.run(draft);
      selectedId = current.requirement.id;
      show("TypeScript通过，宿主验证生产JS/Slint…");
      const checked = await plugin.packages.validate(result.directory);
      if (current.cancelled) throw Error("任务已取消");
      candidate = checked;
      const validation = { digest: checked.digest, manifest: checked.manifest, checks: checked.checks };
      publish(JSON.stringify({ candidate: result.directory, revision: result.revision, validation }));
      show(`候选已通过宿主校验 ${checked.digest}；可试载或明确安装`);
    } catch (error) {
      candidate = undefined;
      if (current.cancelled) show("生成已取消，子进程已退出");
      else fail(error);
    } finally {
      busy = false;
    }
  };

  const stopTrial = async () => {
    if (!trial) return;
    await trial.stop(); // waits until the host confirmed cleanup
    await trial.dispose();
    trial = undefined;
    show("试载已清理");
  };

  /** Install or roll back; only ever on an explicit click, and never resubmitted. */
  const manage = async (rollback: boolean) => {
    if (!idle()) throw Error("请先等待任务完成并停止试载");
    if (!rollback && !candidate) throw Error("请先生成并通过宿主校验");
    busy = true;
    publish(`用户明确${rollback ? "回退" : "安装"} ${selectedId}`);
    let current: PackageChange;
    try {
      current = rollback ? await plugin.packages.rollback(selectedId) : await candidate!.install();
    } catch (error) {
      busy = false;
      throw error;
    }
    change = current;
    const report = () => {
      const text = `包操作：${current.status}${current.message ? " / " + current.message : ""}`;
      publish(text);
      show(text);
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
    show("需求已读取，点击生成");
  });
  window.on("generate", generate);
  window.on("cancelTask", async () => {
    if (job && busy && !change && !starting) {
      job.cancel();
      await job.agent?.waitForIdle();
      show("生成取消已请求，等待任务清理");
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
    publish(JSON.stringify({ trial: trial.status }));
    show(`试载：${trial.status}；请在独立计数器窗口测试增加/归零`);
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

  plugin.commands.add({ id: "workshop.open", path: ["插件", "插件工坊…"], run: () => window.show() });
}
