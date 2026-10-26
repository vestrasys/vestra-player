import type { PackageChange, Plugin, PluginInfo } from "@vestra-player/sdk";
import { bytes, failure, sourceCode, sourceUrl } from "./labels.ts";
import { directorySize, timing } from "./sizes.ts";

/**
 * What the page shows, as codes and data: ui/plugins.slint turns the codes
 * into words. `version` is "" for a development load, `origin` a
 * `sourceCode`, `state` a `PluginInfo["state"]`.
 */
type Row = { id: string; version: string; origin: string; state: string; size: string; dim: boolean; failed: boolean; selected: boolean };
/**
 * One detail line: `label` a code (`failure`, `warning`, `note`, `source`,
 * `package`, `data`, `capabilities`, "" for a continued list);
 * `kind` says how the page words `value`: "" shows it as it is, `source`
 * is a source code (with `url`), `capability` a capability id, the others
 * (`development`, `self`, `builtin`, `unavailable`, `none`) fixed notes.
 */
type Detail = { label: string; kind: string; value: string; url: string; warn: boolean };
/**
 * The message line: `kind` picks the page's sentence, `id` the plugin,
 * `code` a change status or failure code, `text` data (a version, the
 * player's message).
 */
type Message = { kind: string; id?: string; code?: string; text?: string };

/** The J07 page in the options window (category Plugins). */
export const PLUGINS_PAGE = "plugins.manage";

/**
 * Plugin Manager (J07), a page of the options window: every plugin the player knows (running, installed but
 * disabled or failed, development loads) from `packages.list`,
 * following `packages.onChange`. The selected plugin can be disabled or
 * enabled, reloaded, uninstalled (asks, optionally with its data) and its
 * data folder opened. Its own row cannot be disabled; built-in plugins come
 * with the player and can be neither uninstalled nor disabled (a disabled
 * one can still be enabled).
 */
export class PluginsPage {
  readonly #plugin: Plugin;
  readonly #page;
  #list: PluginInfo[] = [];
  #selected: string | null = null;
  /** Ids with a call in flight (buttons off, tip 「Updating」). */
  readonly #busy = new Set<string>();
  #message: Message & { warn: boolean } = { kind: "", warn: false };
  /** Data directory → bytes (`null`: missing or too slow). Keyed by directory: a replaced plugin gets a new one. */
  readonly #sizes = new Map<string, number | null>();
  #measuring: ReturnType<typeof setTimeout> | null = null;
  #measureRun = 0;

  constructor(plugin: Plugin) {
    this.#plugin = plugin;
    const page = (this.#page = plugin.settings.page("ui/plugins.slint", {
      id: PLUGINS_PAGE,
      category: "plugins",
      title: "Plugin Manager",
      order: 10,
    }));
    page.on("select", (id: string) => {
      if (id === this.#selected) return;
      this.#selected = id;
      this.#say({ kind: "" });
    });
    page.on("toggle", () => this.#toggle());
    page.on("reload", () => this.#reload());
    page.on("uninstall", () => this.#uninstall());
    page.on("openData", () => this.#openData());
    page.bind(() => this.#props());
    plugin.packages.onChange((list) => this.#accept(list));
    plugin.onDispose(() => {
      if (this.#measuring) clearTimeout(this.#measuring);
    });
  }

  /** Read the list again and measure the data folders afresh. */
  async refresh() {
    try {
      this.#accept(await this.#plugin.packages.list());
    } catch (error) {
      this.#say({ kind: "list-failed", ...failure(error) }, true);
    }
    this.#sizes.clear();
    this.#measureSoon(0);
  }

  /** Settings ▸ Plugin Manager…: the options window on this page, list and sizes refreshed. */
  async show() {
    await this.refresh();
    await this.#plugin.settings.show(PLUGINS_PAGE);
  }

  #accept(list: PluginInfo[]) {
    this.#list = [...list].sort((a, b) => a.id.localeCompare(b.id));
    if (this.#selected && !this.#list.some((p) => p.id === this.#selected)) this.#selected = null;
    // New plugins (or a new data directory) are measured shortly after.
    if (this.#list.some((p) => p.dataDirectory && !this.#sizes.has(p.dataDirectory))) this.#measureSoon(timing.sizeDebounce);
    this.#plugin.update();
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
    for (const { dataDirectory: directory } of this.#list) {
      if (!directory || this.#sizes.has(directory)) continue;
      const size = await directorySize(directory);
      if (run !== this.#measureRun) return;
      this.#sizes.set(directory, size);
      this.#plugin.update();
    }
  }

  #say(message: Message, warn = false) {
    this.#message = { ...message, warn };
  }

  #info() {
    return this.#list.find((p) => p.id === this.#selected);
  }

  /** The data size column: 「…」 until measured, 「—」 without a directory. */
  #size(info: PluginInfo) {
    const directory = info.dataDirectory;
    if (!directory) return bytes(null);
    return this.#sizes.has(directory) ? bytes(this.#sizes.get(directory)!) : "…";
  }

  #props() {
    const info = this.#info();
    const self = info?.id === this.#plugin.id;
    const busy = !!info && (this.#busy.has(info.id) || info.state === "starting");
    const installed = info?.origin === "installed";
    const canAct = !!info && !busy;
    // Built-ins cannot be disabled; one that is disabled can still be enabled.
    const builtinOn = !!info?.builtin && info.state !== "disabled";
    return {
      rows: this.#list.map(
        (p): Row => ({
          id: p.id,
          version: p.version ?? "",
          origin: sourceCode(p),
          state: p.state,
          size: this.#size(p),
          dim: p.state === "disabled",
          // Failed, or running with a warning: the state is drawn in the accent colour.
          failed: p.state === "failed" || !!p.error,
          selected: p.id === this.#selected,
        }),
      ),
      hasSelection: !!info,
      details: info ? this.#details(info) : [],
      toggleEnables: info?.state === "disabled",
      canToggle: canAct && !self && !builtinOn,
      // Tips are codes: self, busy, builtin, not-installed.
      toggleTip: self ? "self" : busy ? "busy" : builtinOn ? "builtin" : "",
      canReload: canAct && !self && info.state !== "disabled",
      canUninstall: canAct && !self && installed && !info.builtin,
      uninstallTip: self ? "self" : busy ? "busy" : info?.builtin ? "builtin" : info && !installed ? "not-installed" : "",
      canOpen: !!info?.dataDirectory,
      message: { id: "", code: "", text: "", ...this.#message },
    };
  }

  #details(info: PluginInfo): Detail[] {
    const lines: Detail[] = [];
    const line = (label: string, kind: string, value = "", warn = false, url = "") => lines.push({ label, kind, value, url, warn });
    // A running plugin's `error` is a warning: the player refused its panel
    // because another plugin holds that position (the player's own message).
    if (info.error) line(info.state === "failed" ? "failure" : "warning", "", info.error, true);
    if (info.origin === "development") line("note", "development");
    if (info.id === this.#plugin.id) line("note", "self");
    else if (info.builtin) line("note", "builtin");
    // A source kind, or (development loads, older players) the origin.
    line("source", "source", info.source?.kind ?? info.origin, false, sourceUrl(info));
    line("package", "", info.directory);
    if (info.dataDirectory) line("data", "", info.dataDirectory);
    else line("data", "unavailable");
    info.capabilities.forEach((capability, i) => line(i === 0 ? "capabilities" : "", "capability", capability));
    if (info.capabilities.length === 0) line("capabilities", "none");
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
      this.#say({ kind: "failure", ...failure(error, info) }, true);
      // The list may be out of date (a plugin gone, or built in after all): read it again.
      const code = (error as { code?: string }).code;
      if (code === "unsupported" || code === "missing") await this.#relist();
    } finally {
      this.#busy.delete(info.id);
      this.#plugin.update();
    }
  }

  async #relist() {
    try {
      this.#accept(await this.#plugin.packages.list());
    } catch {
      // Keep the old list; packages.onChange brings the next one.
    }
  }

  /** A finished action: on the message line and as an OSD (`shown`, the same sentence). */
  #done(message: Message, shown: Promise<void>) {
    this.#say(message);
    // Headless hosts refuse the display; that never fails the action itself.
    void shown.catch(() => {});
  }

  async #toggle() {
    const info = this.#info();
    if (!info || info.id === this.#plugin.id) return;
    const { id } = info;
    if (info.state === "disabled")
      return this.#busyWith(info, async () => {
        await this.#plugin.packages.setEnabled(id, true);
        this.#done({ kind: "enabled", id }, this.#plugin.osd.show("Enabled {}", [id]));
      });
    if (info.builtin) return;
    const answer = await this.#ask(() =>
      this.#plugin.dialogs.confirm({
        message: "Disable {}?",
        args: [id],
        detail:
          info.origin === "development"
            ? "Its menus, shortcuts, windows and effects are removed at once; open media keeps playing. A development-loaded plugin stays disabled only for this run."
            : "Its menus, shortcuts, windows and effects (such as the equalizer, filters, subtitle style) are removed at once; open media keeps playing. It stays disabled after Vestra Player restarts; you can enable it here at any time.",
        confirm: "Disable",
      }),
    );
    if (!answer?.confirmed) return;
    return this.#busyWith(info, async () => {
      await this.#plugin.packages.setEnabled(id, false);
      this.#done({ kind: "disabled", id }, this.#plugin.osd.show("Disabled {}", [id]));
    });
  }

  /** Reload: stop and start again (recovering a plugin that went wrong). */
  #reload() {
    const info = this.#info();
    if (!info || info.id === this.#plugin.id) return;
    const { id } = info;
    return this.#busyWith(info, async () => {
      await this.#plugin.packages.setEnabled(id, false);
      await this.#plugin.packages.setEnabled(id, true);
      this.#done({ kind: "reloaded", id }, this.#plugin.osd.show("Reloaded {}", [id]));
    });
  }

  async #uninstall() {
    const info = this.#info();
    if (!info || info.origin !== "installed" || info.builtin || info.id === this.#plugin.id) return;
    const { id } = info;
    const answer = await this.#ask(() =>
      this.#plugin.dialogs.confirm({
        message: "Uninstall {}?",
        args: [id],
        detail: "It is removed from the installed plugins. Its shortcut changes are kept and come back if you install it again.",
        confirm: "Uninstall",
        destructive: true,
        checkbox: "Also delete its settings and data?",
      }),
    );
    if (!answer?.confirmed) return;
    return this.#busyWith(info, async () => {
      const change = await this.#plugin.packages.uninstall(id, { removeData: answer.checked });
      const status = await this.#follow(change, "uninstall", id);
      if (status !== "committed") return this.#say({ kind: "uninstall", id, code: status, text: change.message ?? "" }, true);
      if (info.dataDirectory) this.#sizes.delete(info.dataDirectory);
      if (this.#selected === id) this.#selected = null;
      this.#done({ kind: "uninstalled", id }, this.#plugin.osd.show("Uninstalled {}", [id]));
    });
  }

  /** Show an uninstall's progress until it ends; its final status. */
  async #follow(change: PackageChange, kind: string, id: string) {
    const show = (status: PackageChange["status"]) => this.#say({ kind, id, code: status });
    show(change.status);
    const watch = change.onChange(show);
    try {
      return await change.done;
    } finally {
      await watch.dispose();
    }
  }

  async #ask(confirm: () => ReturnType<Plugin["dialogs"]["confirm"]>) {
    try {
      return await confirm();
    } catch (error) {
      this.#say({ kind: "confirm-failed", text: (error as Error).message }, true);
      return null;
    }
  }

  async #openData() {
    const info = this.#info();
    const directory = info?.dataDirectory;
    if (!directory) return;
    try {
      await this.#plugin.system.openPath(directory);
      this.#say({ kind: "" });
    } catch (error) {
      const code = (error as { code?: string }).code;
      if (code === "missing") this.#say({ kind: "no-data-folder" }, true);
      else this.#say({ kind: "data-folder-failed", text: (error as Error).message }, true);
    }
  }
}
