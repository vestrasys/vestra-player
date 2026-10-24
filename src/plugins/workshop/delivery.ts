// Trying and installing a session's plugin in the player (plugins.manage):
// pack → packages.validateArchive (the player's checks, Slint included) →
// a trial load, or an install the user confirmed, with rollback. Used by the
// agent's trial_load / install_plugin and by the result card's buttons.
import type { PackageCandidate, PackageChange, PackageTrial, Plugin, PluginError } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { TRIAL_CAPABILITIES } from "./prompt.ts";
import type { SessionPaths, ToolResult, Toolchain } from "./toolchain.ts";

/** How long a trial may take to activate before it is reported as not started. */
export const TRIAL_ACTIVATION_MS = 15_000;

const CHANGE_TEXT: Record<string, string> = {
  committed: "完成",
  cancelled: "已取消",
  failed: "失败",
  failedDegraded: "失败，未能完全恢复原版本",
  tooLate: "已无法取消",
};

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const code = (error: unknown) => (error as PluginError)?.code;

export interface TrialState {
  id: string;
  name: string;
  handle: PackageTrial;
}

export class Delivering {
  private readonly plugin: Plugin;
  private readonly toolchain: Toolchain;
  private readonly owner: () => string;
  /** The running trial (one at a time). */
  trial: TrialState | null = null;
  /** Ids installed during this run: 「回退」 is offered for them. */
  readonly installed = new Set<string>();
  /** The result card's line: what the last action did. */
  status = "";
  busy = false;

  constructor(plugin: Plugin, toolchain: Toolchain, owner: () => string) {
    this.plugin = plugin;
    this.toolchain = toolchain;
    this.owner = owner;
  }

  private say(text: string) {
    this.status = text;
    this.plugin.update();
  }

  /** One action at a time; the card's buttons are disabled meanwhile. */
  private async exclusive(work: () => Promise<ToolResult>): Promise<ToolResult> {
    if (this.busy) return { ok: false, text: "另一个试载或安装正在进行，请稍候" };
    this.busy = true;
    this.plugin.update();
    try {
      return await work();
    } finally {
      this.busy = false;
      this.plugin.update();
    }
  }

  /** Packs the project and lets the player validate the package. */
  private async candidate(session: SessionPaths, id: string, signal?: AbortSignal): Promise<{ candidate?: PackageCandidate; failure?: ToolResult }> {
    this.say(`正在打包 ${id}…`);
    const packed = await this.toolchain.pack(session, id, this.owner(), signal);
    if (!packed.ok || !packed.file) return { failure: packed };
    this.say(`正在由播放器校验 ${id}…`);
    try {
      return { candidate: await this.plugin.packages.validateArchive(packed.file) };
    } catch (error) {
      // Manifest, package and Slint problems the player found.
      return { failure: { ok: false, text: `播放器校验没有通过（${code(error) ?? "error"}）：${errorText(error)}` } };
    }
  }

  async stopTrial(): Promise<void> {
    const trial = this.trial;
    if (!trial) return;
    try {
      await trial.handle.stop(); // waits until the player confirmed cleanup
    } catch (error) {
      if (code(error) === "cleanup") {
        this.say("播放器还没有确认试载已清理，请再点一次「停止试载」。");
        throw new Error("播放器未确认试载已清理，请再次停止试载。");
      }
      throw error;
    }
    await trial.handle.dispose();
    this.trial = null;
    this.say(`已停止试载 ${trial.name}。`);
  }

  /**
   * The trial's run state from the plugin list: the trial handle reports
   * only the player's first answer (see the report, R-j), the list shows
   * whether activation succeeded or failed.
   */
  private async activation(id: string): Promise<{ state: string; error: string | null }> {
    const deadline = Date.now() + TRIAL_ACTIVATION_MS;
    let seen = false;
    for (;;) {
      const entry = (await this.plugin.packages.list()).find((p) => p.id === id && p.origin === "trial");
      if (entry) {
        seen = true;
        if (entry.state !== "starting") return { state: entry.state, error: entry.error };
      } else if (seen) return { state: "failed", error: null }; // stopped by the player after a failure
      if (Date.now() > deadline) return { state: seen ? "starting" : "missing", error: null };
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  trialLoad(session: SessionPaths, id: string, signal?: AbortSignal): Promise<ToolResult> {
    return this.exclusive(async () => {
      if (this.trial) await this.stopTrial();
      const directory = await this.toolchain.project(session, id);
      const manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
      const name = String(manifest.name ?? id);
      const other = (manifest.capabilities ?? []).filter((c: string) => !TRIAL_CAPABILITIES.includes(c));
      if (other.length) {
        const text = `不能试载：播放器的试载只支持 ${TRIAL_CAPABILITIES.join("、")}，这个插件还用到 ${other.join("、")}。可以直接安装（之后能回退）。`;
        this.say(text);
        return { ok: false, text };
      }
      const { candidate, failure } = await this.candidate(session, id, signal);
      if (!candidate) {
        this.say(failure!.text.split("\n")[0]);
        return failure!;
      }
      this.say(`正在试载 ${name}…`);
      let handle: PackageTrial;
      try {
        handle = await candidate.trial();
      } catch (error) {
        const text = `播放器没有开始试载（${code(error) ?? "error"}）：${errorText(error)}`;
        this.say(text);
        return { ok: false, text };
      }
      const { state, error } = await this.activation(id);
      if (state === "running") {
        this.trial = { id, name, handle };
        const text = `已试载：${name}（${id} ${candidate.manifest.version}）正在播放器里运行，可以去试用了。用完在 AI 窗口点「停止试载」。`;
        this.say(`试载中：${name}`);
        return { ok: true, text };
      }
      await handle.dispose(); // stops and cleans up what was started
      const text =
        state === "failed"
          ? `试载的插件没能启动：${error ?? "播放器没有给出原因（激活失败）"}`
          : state === "starting"
            ? `试载的插件 ${TRIAL_ACTIVATION_MS / 1000} 秒内没有启动完（activate 没有结束），已停止试载。`
            : "试载没有出现在播放器的插件列表里（多半是激活失败），已停止试载。";
      this.say(text);
      return { ok: false, text };
    });
  }

  /** Follows an install or rollback to its end. */
  private async follow(change: PackageChange, action: string): Promise<{ ok: boolean; text: string }> {
    const listening = change.onChange((status) => this.say(`${action}：${status}…`));
    try {
      const status = await change.done;
      const text = `${action}${CHANGE_TEXT[status] ?? status}${change.message ? `（${change.message}）` : ""}`;
      return { ok: status === "committed", text };
    } finally {
      await listening.dispose();
    }
  }

  /**
   * Installs the project. `ask`: the agent asked, so the user confirms in a
   * dialog first (a click on 「安装」 is the user's own decision). Another
   * source's plugin of the same id is replaced only after asking.
   */
  install(session: SessionPaths, id: string, ask: boolean, signal?: AbortSignal): Promise<ToolResult> {
    return this.exclusive(async () => {
      const directory = await this.toolchain.project(session, id);
      const manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
      const name = String(manifest.name ?? id);
      if (ask) {
        const answer = await this.plugin.dialogs.confirm({
          message: `安装 AI 生成的插件「${name}」？`,
          detail: `${id} ${manifest.version}\n能力：${(manifest.capabilities ?? []).join("、") || "无"}\n安装后可以在 AI 窗口里「回退」。`,
          confirm: "安装",
        });
        if (!answer.confirmed) {
          this.say("没有安装（已取消）。");
          return { ok: false, text: "用户取消了安装" };
        }
      }
      // The player refuses to install a package while its trial runs.
      if (this.trial?.id === id) await this.stopTrial();
      const { candidate, failure } = await this.candidate(session, id, signal);
      if (!candidate) {
        this.say(failure!.text.split("\n")[0]);
        return failure!;
      }
      this.say(`正在安装 ${name}…`);
      const options = { source: { kind: "workshop" as const } };
      let change: PackageChange;
      try {
        change = await candidate.install(options);
      } catch (error) {
        if (code(error) === "unsupported") {
          const text = `${id} 和播放器内置插件同名，不能安装，请换一个 id。`;
          this.say(text);
          return { ok: false, text };
        }
        if (code(error) !== "conflict") throw error;
        const answer = await this.plugin.dialogs.confirm({
          message: `替换已安装的 ${id}？`,
          detail: "它是从别处安装的（不是 Vestra Player AI 生成的）。替换后新插件使用新的空数据目录；原插件的数据保留，以后从原来源重新安装时接上。",
          confirm: "替换",
          destructive: true,
        });
        if (!answer.confirmed) {
          this.say("没有安装：保留了已安装的插件。");
          return { ok: false, text: `用户没有替换从别处安装的 ${id}` };
        }
        change = await candidate.install({ ...options, replace: true });
      }
      const { ok, text } = await this.follow(change, `安装 ${name} ${manifest.version}`);
      if (ok) this.installed.add(id);
      this.say(ok ? `已安装 ${name} ${manifest.version}，可以「回退」。` : text);
      return { ok, text: ok ? `${text}。插件已在播放器里启用；用户可以在 AI 窗口「回退」到之前的状态。` : text };
    });
  }

  /**
   * Undoes this run's install: back to the previous version, or, for a
   * first install, removes the plugin again (its data stays, so a later
   * install from here reconnects to it).
   */
  rollback(id: string): Promise<ToolResult> {
    return this.exclusive(async () => {
      const entry = (await this.plugin.packages.list()).find((p) => p.id === id && p.origin === "installed");
      if (!entry) {
        this.installed.delete(id);
        this.say(`${id} 已经不在已安装的插件里。`);
        return { ok: false, text: `${id} 没有安装` };
      }
      this.say(`正在回退 ${id}…`);
      const change = entry.previousVersion ? await this.plugin.packages.rollback(id) : await this.plugin.packages.uninstall(id);
      const { ok, text } = await this.follow(change, entry.previousVersion ? `回退到 ${entry.previousVersion}` : `卸载 ${id}`);
      if (ok) this.installed.delete(id);
      this.say(text);
      return { ok, text };
    });
  }
}
