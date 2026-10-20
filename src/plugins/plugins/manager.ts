import { dirname, join } from "node:path";
import type { PackageChange, Plugin, PluginInfo } from "../../sdk/plugin/index.ts";
import { bytes, CAPABILITIES, CHANGE, failureText, ORIGIN, STATE } from "./labels.ts";
import { directorySize, timing } from "./sizes.ts";

type Row = { id: string; version: string; origin: string; state: string; size: string; dim: boolean; failed: boolean; selected: boolean };
type Detail = { label: string; text: string; warn: boolean };

const SELF_TIP = "插件管理不能停用自身";
const BUSY_TIP = "正在更新";

/**
 * 插件管理 (J07): every plugin the player knows (running, installed but
 * disabled or failed, development loads, trials) from `packages.list`,
 * following `packages.onChange`. The selected plugin can be disabled or
 * enabled, reloaded, rolled back, uninstalled (asks, optionally with its
 * data) and its data folder opened. Its own row cannot be disabled.
 */
export class PluginsWindow {
  readonly #plugin: Plugin;
  readonly #osd: (text: string) => void;
  readonly #window;
  /** The data root: `<root>/<id>` is each plugin's data folder (API reference §3.4). */
  readonly #root: string | null;
  #list: PluginInfo[] = [];
  #selected: string | null = null;
  /** Ids with a call in flight (buttons off, 「正在更新」). */
  readonly #busy = new Set<string>();
  #message = { text: "", warn: false };
  readonly #sizes = new Map<string, number | null>();
  #measuring: ReturnType<typeof setTimeout> | null = null;
  #measureRun = 0;

  constructor(plugin: Plugin, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#osd = osd;
    let root: string | null = null;
    try {
      root = dirname(plugin.dataDirectory);
    } catch {
      // No data directory from the player: sizes 「—」, 打开数据目录 off.
    }
    this.#root = root;
    const window = (this.#window = plugin.windows.create("ui/plugins.slint"));
    window.on("select", (id: string) => {
      if (id === this.#selected) return;
      this.#selected = id;
      this.#say("");
    });
    window.on("toggle", () => this.#toggle());
    window.on("reload", () => this.#reload());
    window.on("rollback", () => this.#rollback());
    window.on("uninstall", () => this.#uninstall());
    window.on("openData", () => this.#openData());
    window.on("close", () => window.close());
    window.bind(() => this.#props());
    plugin.packages.onChange((list) => this.#accept(list));
    plugin.onDispose(() => {
      if (this.#measuring) clearTimeout(this.#measuring);
    });
  }

  /** 设置 ▶ 插件管理…: open (or bring to the front) with a fresh list and sizes. */
  async show() {
    try {
      this.#accept(await this.#plugin.packages.list());
    } catch (error) {
      this.#say(`无法读取插件列表：${failureText(error)}`, true);
    }
    this.#sizes.clear();
    this.#measureSoon(0);
    await this.#window.show();
  }

  #accept(list: PluginInfo[]) {
    const known = new Set(this.#list.map((p) => p.id));
    this.#list = [...list].sort((a, b) => a.id.localeCompare(b.id));
    if (this.#selected && !this.#list.some((p) => p.id === this.#selected)) this.#selected = null;
    // New plugins (or a changed list) are measured shortly after.
    if (this.#window.opened && this.#list.some((p) => !known.has(p.id))) this.#measureSoon(timing.sizeDebounce);
    this.#plugin.update();
  }

  dataDirectory(id: string): string | null {
    return this.#root ? join(this.#root, id) : null;
  }

  #measureSoon(delay: number) {
    if (this.#measuring) clearTimeout(this.#measuring);
    this.#measuring = setTimeout(() => {
      this.#measuring = null;
      void this.#measure();
    }, delay);
  }
  /** Data sizes, one plugin at a time (a newer run abandons an older one). */
  async #measure() {
    const run = ++this.#measureRun;
    for (const info of this.#list) {
      if (this.#sizes.has(info.id)) continue;
      const directory = this.dataDirectory(info.id);
      const size = directory ? await directorySize(directory) : null;
      if (run !== this.#measureRun) return;
      this.#sizes.set(info.id, size);
      this.#plugin.update();
    }
  }

  #say(text: string, warn = false) {
    this.#message = { text, warn };
  }

  #info() {
    return this.#list.find((p) => p.id === this.#selected);
  }

  #props() {
    const info = this.#info();
    const self = info?.id === this.#plugin.id;
    const busy = !!info && (this.#busy.has(info.id) || info.state === "starting");
    const trial = info?.origin === "trial";
    const installed = info?.origin === "installed";
    const canAct = !!info && !busy && !trial;
    return {
      rows: this.#list.map(
        (p): Row => ({
          id: p.id,
          version: p.version ?? "开发",
          origin: ORIGIN[p.origin] ?? p.origin,
          state: STATE[p.state] ?? p.state,
          size: this.#sizes.has(p.id) ? bytes(this.#sizes.get(p.id)!) : "…",
          dim: p.state === "disabled",
          failed: p.state === "failed",
          selected: p.id === this.#selected,
        }),
      ),
      hasSelection: !!info,
      details: info ? this.#details(info) : [],
      toggleText: info?.state === "disabled" ? "启用" : "停用",
      canToggle: canAct && !self,
      toggleTip: self ? SELF_TIP : busy ? BUSY_TIP : "",
      canReload: canAct && !self && info.state !== "disabled",
      canRollback: canAct && !self && installed && !!info.previousVersion,
      canUninstall: canAct && !self && installed,
      uninstallTip: self ? SELF_TIP : busy ? BUSY_TIP : info && !installed ? "只能卸载已安装的插件" : "",
      canOpen: !!info && !!this.#root && !trial,
      message: this.#message.text,
      messageWarn: this.#message.warn,
    };
  }

  #details(info: PluginInfo): Detail[] {
    const lines: Detail[] = [];
    const line = (label: string, text: string, warn = false) => lines.push({ label, text, warn });
    if (info.error) line("失败原因", info.error, true);
    if (info.origin === "development") line("说明", "开发加载：停用只在本次运行有效；不能卸载或回退");
    if (info.origin === "trial") line("说明", "试载中的插件由插件工坊管理");
    if (info.id === this.#plugin.id) line("说明", SELF_TIP);
    if (info.previousVersion) line("上一版本", info.previousVersion);
    line("包目录", info.directory);
    line("数据目录", this.dataDirectory(info.id) ?? "不可用");
    info.capabilities.forEach((capability, i) =>
      line(i === 0 ? "能力" : "", `${capability}  ${CAPABILITIES[capability] ?? ""}`.trimEnd()),
    );
    if (info.capabilities.length === 0) line("能力", "（无）");
    return lines;
  }

  /** Run `action` for the selected plugin with its buttons off; report failures. */
  async #busyWith(info: PluginInfo, action: () => Promise<void>) {
    if (this.#busy.has(info.id)) return;
    this.#busy.add(info.id);
    this.#plugin.update();
    try {
      await action();
    } catch (error) {
      this.#say(failureText(error, info), true);
    } finally {
      this.#busy.delete(info.id);
      this.#plugin.update();
    }
  }

  #done(text: string) {
    this.#say(text);
    this.#osd(text);
  }

  async #toggle() {
    const info = this.#info();
    if (!info || info.id === this.#plugin.id) return;
    if (info.state === "disabled")
      return this.#busyWith(info, async () => {
        await this.#plugin.packages.setEnabled(info.id, true);
        this.#done(`已启用 ${info.id}`);
      });
    const answer = await this.#ask({
      message: `停用 ${info.id}？`,
      detail:
        info.origin === "development"
          ? "它的菜单、快捷键、窗口与处理效果会立即撤销，打开的媒体继续播放。开发加载的插件只在本次运行内停用。"
          : "它的菜单、快捷键、窗口与处理效果（如均衡器、滤镜、字幕样式）会立即撤销，打开的媒体继续播放。重启 Vestra 后仍保持停用，可随时在这里启用。",
      confirm: "停用",
    });
    if (!answer?.confirmed) return;
    return this.#busyWith(info, async () => {
      await this.#plugin.packages.setEnabled(info.id, false);
      this.#done(`已停用 ${info.id}`);
    });
  }

  /** 重新加载: stop and start again (recovering a plugin that went wrong). */
  #reload() {
    const info = this.#info();
    if (!info || info.id === this.#plugin.id) return;
    return this.#busyWith(info, async () => {
      await this.#plugin.packages.setEnabled(info.id, false);
      await this.#plugin.packages.setEnabled(info.id, true);
      this.#done(`已重新加载 ${info.id}`);
    });
  }

  #rollback() {
    const info = this.#info();
    if (!info?.previousVersion || info.id === this.#plugin.id) return;
    const target = info.previousVersion;
    return this.#busyWith(info, async () => {
      const change = await this.#plugin.packages.rollback(info.id);
      const status = await this.#follow(change, `回退 ${info.id}`);
      if (status === "committed") this.#done(`已回退 ${info.id} 到 ${target}`);
      else this.#say(`回退 ${info.id}：${CHANGE[status] ?? status}${change.message ? `（${change.message}）` : ""}`, true);
    });
  }

  async #uninstall() {
    const info = this.#info();
    if (!info || info.origin !== "installed" || info.id === this.#plugin.id) return;
    const answer = await this.#ask({
      message: `卸载 ${info.id}？`,
      detail: "将从安装库移除它的当前版本与上一版本。它的快捷键修改会保留，重新安装后恢复。",
      confirm: "卸载",
      destructive: true,
      checkbox: "同时删除它的设置与数据",
    });
    if (!answer?.confirmed) return;
    return this.#busyWith(info, async () => {
      const change = await this.#plugin.packages.uninstall(info.id, { removeData: answer.checked });
      const status = await this.#follow(change, `卸载 ${info.id}`);
      if (status !== "committed")
        return this.#say(`卸载 ${info.id}：${CHANGE[status] ?? status}${change.message ? `（${change.message}）` : ""}`, true);
      this.#sizes.delete(info.id);
      if (this.#selected === info.id) this.#selected = null;
      this.#done(`已卸载 ${info.id}`);
    });
  }

  /** Show a change's progress until it ends; its final status. */
  async #follow(change: PackageChange, what: string) {
    const show = (status: PackageChange["status"]) => this.#say(`${what}：${CHANGE[status] ?? status}`);
    show(change.status);
    const watch = change.onChange(show);
    try {
      return await change.done;
    } finally {
      await watch.dispose();
    }
  }

  async #ask(options: Parameters<Plugin["dialogs"]["confirm"]>[0]) {
    try {
      return await this.#plugin.dialogs.confirm(options);
    } catch (error) {
      this.#say(`无法确认：${(error as Error).message}`, true);
      return null;
    } finally {
      // The alert is a sheet on the main window, which comes to the front;
      // bring this window back over it.
      this.#raise();
    }
  }

  #raise() {
    if (this.#window.opened) void this.#window.show().catch(() => {});
  }

  async #openData() {
    const info = this.#info();
    const directory = info && this.dataDirectory(info.id);
    if (!directory) return;
    try {
      await this.#plugin.system.openPath(directory);
      this.#say("");
    } catch (error) {
      const code = (error as { code?: string }).code;
      this.#say(code === "missing" ? "数据目录不存在（插件还没有保存过设置或数据）" : `数据目录不可用：${(error as Error).message}`, true);
    }
  }
}
