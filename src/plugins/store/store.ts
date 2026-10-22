import { readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type {
  ConfirmOptions,
  ImageRef,
  JsonStore,
  MarkdownRef,
  Plugin,
  PluginInfo,
  Scope,
  Window,
} from "@vestra-player/sdk";
import { buildItems, checkVersion, isInstalledRow, updateCount, type Display, type Item } from "./catalog.ts";
import { clean } from "./index-format.ts";
import { Icons } from "./icons.ts";
import * as installer from "./installer.ts";
import { cut, day, hostOf, permissions, sourceName, TRUST_NOTE } from "./labels.ts";
import { get } from "./net.ts";
import { Revocations, revocationKey } from "./revocations.ts";
import { Repos, type Deps } from "./repos.ts";
import { compareVersions, isVersion } from "./semver.ts";
import { SettingsPage } from "./settings-page.ts";

/** Persisted in `<data>/settings.json`. */
export type Settings = {
  /** Third-party repository addresses (normalized). */
  repos: string[];
  autoCheck: boolean;
  /** When the last automatic or manual full check finished. */
  lastCheck: number;
  /** `revocationKey(...)` → handled: auto-disabled once, or the user accepted the risk. */
  revocations: Record<string, "disabled" | "accepted">;
};

const DEFAULTS: Settings = { repos: [], autoCheck: true, lastCheck: 0, revocations: {} };

const validSettings = (value: unknown): value is Settings => {
  const v = value as Settings;
  return (
    typeof v === "object" &&
    v !== null &&
    Array.isArray(v.repos) &&
    v.repos.every((r) => typeof r === "string") &&
    typeof v.autoCheck === "boolean" &&
    typeof v.lastCheck === "number" &&
    typeof v.revocations === "object" &&
    v.revocations !== null
  );
};

/** Delays of the automatic check (design §3.6, decision 2.2-6); tests shorten them. */
export const timing = {
  /** After activation, once. */
  firstCheck: 30_000,
  /** How often to look whether a day has passed. */
  tick: 60 * 60 * 1000,
  /** Then every 24 hours. */
  interval: 24 * 60 * 60 * 1000,
};

export const MENU_ORDER = 822;
const README_LIMIT = 256 * 1024;
const FILTERS = 3;
const SORTS = ["排序：名称", "排序：更新时间"];
const REMOTE_IMAGE = /!\[[^\]]*\]\(\s*<?(https?:\/\/[^)\s>]+)/g;
const MANAGE_NOTE = "停用、重新加载、查看数据目录：设置 ▶ 插件管理…";

/** Rough label width for the 11px buttons (CJK ≈ 11px, other ≈ 6.5px, padding 16). */
export function labelWidth(text: string, min = 66): number {
  let width = 16;
  for (const ch of text) width += /[⺀-￿]/.test(ch) ? 11 : 6.5;
  return Math.max(min, Math.ceil(width));
}

type Doc = {
  key: string;
  ref: MarkdownRef | null;
  note: string;
  blocked: number;
  failed: boolean;
  scope: Scope | null;
  /** The document is on another host than a third-party index: not fetched until asked. */
  offHost?: string;
};

/**
 * The store: repositories and the install library combined into rows, the
 * tool window, the menu entry with the update count, the automatic check,
 * dropped packages and the settings page. Installing is in installer.ts,
 * revocations in revocations.ts.
 */
export class Store {
  readonly plugin: Plugin;
  readonly deps: Deps;
  readonly repos: Repos;
  readonly settings: JsonStore<Settings>;
  readonly window: Window;
  readonly icons: Icons;
  readonly revocations: Revocations;
  readonly version: string;
  page!: SettingsPage;
  packages: PluginInfo[] = [];
  items: Item[] = [];

  // Window state
  filter = 0;
  search = "";
  tag: string | null = null;
  sort = 0;
  selected: string | null = null;
  tab = 0;
  linkOpen = false;
  linkText = "";
  #message: { text: string; warn: boolean } | null = null;
  #operation: { text: string; cancel: (() => void) | null } | null = null;
  #doc: Doc = { key: "", ref: null, note: "", blocked: 0, failed: false, scope: null };
  #docAbort: AbortController | null = null;
  readonly #texts = new Map<string, string>();
  readonly #remoteShown = new Set<string>();
  readonly #images = new Map<string, ImageRef>();
  readonly #displays = new Map<string, Display>();
  readonly #logo: string;
  // Menu
  #menuScope: Scope | null = null;
  #menuCount = -1;
  #menuChain: Promise<void> = Promise.resolve();
  #announced = false;
  #disposed = false;

  constructor(plugin: Plugin, deps: Omit<Deps, "userAgent">) {
    this.plugin = plugin;
    this.version = ownVersion();
    this.deps = { ...deps, userAgent: `VestraPlayer/${plugin.app.version} vestra.store/${this.version}` };
    this.settings = plugin.storage.json("settings", { defaults: DEFAULTS, validate: validSettings });
    this.repos = new Repos(this.deps, plugin.dataDirectory);
    this.repos.sync(this.settings.value.repos);
    this.icons = new Icons(plugin.dataDirectory, deps.fetch, () => this.changed());
    this.revocations = new Revocations(this);
    this.#logo = fileURLToPath(new URL("./assets/icon.png", import.meta.url));
    // Downloads left over from a crash.
    rmSync(join(plugin.dataDirectory, "downloads"), { recursive: true, force: true });

    const window = (this.window = plugin.windows.create("ui/store.slint"));
    window.on("search", (text: string) => {
      this.search = text;
    });
    window.on("setFilter", (index: number) => {
      this.filter = Math.max(0, Math.min(FILTERS - 1, index));
    });
    window.on("setTag", (index: number) => {
      this.tag = index <= 0 ? null : (this.#tags()[index - 1] ?? null);
    });
    window.on("setSort", (index: number) => {
      this.sort = index === 1 ? 1 : 0;
    });
    window.on("select", (key: string) => {
      if (this.selected === key) return;
      this.selected = key;
      void this.#loadDoc();
    });
    window.on("setTab", (index: number) => {
      this.tab = Math.max(0, Math.min(2, index));
      void this.#loadDoc();
    });
    window.on("act", (action: string) => this.#act(action));
    window.on("bannerAct", (action: string) => this.#bannerAct(action));
    window.on("installVersion", (version: string) => {
      const item = this.selectedItem();
      const v = item?.entry?.versions.find((each) => each.version === version);
      if (item && v) return installer.installFromRepository(this, item, v);
    });
    window.on("openLink", (url: string) => this.#openLink(url));
    window.on("factLink", (index: number) => {
      const fact = this.#facts(this.selectedItem())[index];
      if (fact?.url) return this.#openLink(fact.url);
    });
    window.on("fromFile", () => this.#fromFile());
    window.on("fromLink", () => {
      this.linkOpen = !this.linkOpen;
    });
    window.on("linkAccepted", async (url: string) => {
      const text = url.trim();
      if (!text) return;
      this.linkOpen = false;
      this.linkText = "";
      await installer.installFromLink(this, text);
    });
    window.on("linkCancelled", () => {
      this.linkOpen = false;
    });
    window.on("repos", () => plugin.settings.show(SettingsPage.ID));
    window.on("refresh", () => this.refreshAll(true));
    window.on("cancel", () => this.#operation?.cancel?.());
    window.onClose(() => this.#docAbort?.abort());
    window.bind(() => this.props());

    plugin.packages.onChange((list) => this.#accept(list));
    plugin.drops.provide({
      extensions: ["vxp"],
      drop: async ({ paths }) => {
        await this.open(false);
        for (const path of paths) await installer.installFile(this, path);
      },
    });
    this.page = new SettingsPage(this);
    plugin.onDispose(() => {
      this.#disposed = true;
      this.#docAbort?.abort();
    });
  }

  /** Activation: cached indexes and the install library, the menu, the timers. */
  async start() {
    try {
      this.#accept(await this.plugin.packages.list());
    } catch (error) {
      this.deps.log(`packages.list: ${(error as Error).message}`);
    }
    await this.updateMenu();
    const first = setTimeout(() => void this.autoCheck(), timing.firstCheck);
    const tick = setInterval(() => {
      if (this.deps.now() - this.settings.value.lastCheck >= timing.interval) void this.autoCheck();
    }, timing.tick);
    this.plugin.onDispose(() => {
      clearTimeout(first);
      clearInterval(tick);
    });
  }

  async autoCheck() {
    if (!this.settings.value.autoCheck || this.#disposed) return;
    await this.refreshAll(true);
  }

  /** Refresh every repository (or only stale ones), then recompute everything. */
  async refreshAll(force: boolean) {
    this.#message = null;
    const done = this.repos.refreshAll(force);
    this.changed();
    await done;
    if (this.#disposed) return;
    if (force) this.settings.save({ ...this.settings.value, lastCheck: this.deps.now() });
    this.rebuild(true);
  }

  /** The menu command (and the window button): show the window, refresh stale repositories. */
  async open(refresh = true) {
    await this.window.show();
    if (refresh && this.repos.list.some((repo) => this.repos.stale(repo))) void this.refreshAll(false);
    if (!this.selected) {
      const first = this.visible()[0];
      if (first) this.select(first.key);
    }
  }

  select(key: string) {
    this.selected = key;
    void this.#loadDoc();
    this.changed();
  }

  /** Recompute rows after the repositories or the install library changed. */
  rebuild(afterRefresh = false) {
    this.items = buildItems(this.repos.list, this.packages, this.plugin.app.version, (info) => this.#display(info));
    void this.updateMenu(afterRefresh);
    void this.revocations.check();
    void this.#loadDoc();
    this.changed();
  }

  changed() {
    if (!this.#disposed) this.plugin.update();
  }

  #accept(list: PluginInfo[]) {
    this.packages = list;
    this.rebuild();
  }

  /** 「插件商城…」, with the number of updates in the title (design §1.2). */
  updateMenu(afterRefresh = false): Promise<void> {
    this.#menuChain = this.#menuChain.then(async () => {
      if (this.#disposed) return;
      const count = updateCount(this.items);
      if (count === this.#menuCount) return;
      const before = this.#menuCount;
      this.#menuCount = count;
      await this.#menuScope?.dispose();
      const scope = (this.#menuScope = this.plugin.scope());
      scope.commands.add({
        id: "open",
        path: [count > 0 ? `插件商城（${count} 个可更新）…` : "插件商城…"],
        order: MENU_ORDER,
        group: "windows",
        run: () => this.open(),
      });
      if (afterRefresh && count > 0 && before <= 0 && !this.#announced) {
        this.#announced = true;
        this.osd(`插件商城：${count} 个插件可以更新`);
      }
    });
    return this.#menuChain;
  }

  // ------------------------------------------------------------ operations

  /** One download or install at a time. */
  begin(text: string, cancel: (() => void) | null = null): boolean {
    if (this.#operation) {
      this.report("正在进行另一项安装，请等它结束", true);
      return false;
    }
    this.#operation = { text, cancel };
    this.#message = null;
    this.changed();
    return true;
  }

  progress(text: string, cancel?: (() => void) | null) {
    if (!this.#operation) return;
    this.#operation = { text, cancel: cancel === undefined ? this.#operation.cancel : cancel };
    this.changed();
  }

  end() {
    this.#operation = null;
    this.changed();
  }

  /** Why the last `ask` showed nothing (`busy`, `unavailable`, `unsupported`…). */
  lastDialogError: string | null = null;

  get busy(): boolean {
    return this.#operation !== null;
  }

  /** A result for the user: the status line and an on-screen message. */
  report(text: string, warn = false) {
    this.#message = { text, warn };
    this.osd(text);
    this.changed();
  }

  osd(text: string) {
    void this.plugin.osd.show(cut(text.replace(/\s+/g, " "), 200)).catch(() => {});
  }

  /** `dialogs.confirm` with the limits applied; `null` when no dialog could be shown. */
  async ask(options: ConfirmOptions): Promise<{ confirmed: boolean; checked: boolean } | null> {
    this.lastDialogError = null;
    try {
      return await this.plugin.dialogs.confirm({
        ...options,
        message: cut(options.message, 200),
        ...(options.detail === undefined ? {} : { detail: cut(options.detail, 2000) }),
      });
    } catch (error) {
      const code = (error as { code?: string }).code;
      this.lastDialogError = code ?? "failed";
      if (code === "unsupported") this.report("此系统不支持确认框，无法继续", true);
      else if (code === "busy") this.report("另一个对话框正在打开，请先关闭它", true);
      else this.deps.log(`confirm: ${(error as Error).message}`);
      return null;
    }
  }

  // ------------------------------------------------------------ the window

  selectedItem(): Item | null {
    return this.items.find((item) => item.key === this.selected) ?? null;
  }

  /** Filter, search, tag and sort (design §2.1). */
  visible(): Item[] {
    const query = this.search.trim().toLowerCase();
    let items = this.#filtered().filter((item) =>
      !query
        ? true
        : [item.name, item.id, item.author ?? "", item.punchline ?? "", ...item.tags].some((text) =>
            text.toLowerCase().includes(query),
          ),
    );
    if (this.tag) items = items.filter((item) => item.tags.some((t) => t.toLowerCase() === this.tag!.toLowerCase()));
    const byName = (a: Item, b: Item) => a.name.localeCompare(b.name, "zh") || a.key.localeCompare(b.key);
    return items.sort(
      this.sort === 1 ? (a, b) => (b.published ?? "").localeCompare(a.published ?? "") || byName(a, b) : byName,
    );
  }

  #filtered(): Item[] {
    if (this.filter === 1) return this.items.filter(isInstalledRow);
    if (this.filter === 2) return this.items.filter((item) => item.state === "update");
    return this.items;
  }

  #tags(): string[] {
    const tags = new Map<string, string>();
    for (const item of this.#filtered()) for (const tag of item.tags) if (!tags.has(tag.toLowerCase())) tags.set(tag.toLowerCase(), tag);
    return [...tags.values()].sort((a, b) => a.localeCompare(b, "zh"));
  }

  props() {
    const visible = this.visible();
    const tags = this.#tags();
    const item = this.selectedItem();
    const updates = updateCount(this.items);
    const op = this.#operation;
    const base = {
      filter: this.filter,
      updatesLabel: updates > 0 ? `可更新 ${updates}` : "可更新",
      tagModel: ["标签：全部", ...tags],
      tagIndex: this.tag ? Math.max(0, tags.findIndex((t) => t.toLowerCase() === this.tag!.toLowerCase()) + 1) : 0,
      sortModel: SORTS,
      sortIndex: this.sort,
      rows: visible.map((each) => this.#row(each)),
      emptyText: this.#emptyText(visible),
      linkOpen: this.linkOpen,
      statusText: this.#message?.text ?? this.#repoStatus(),
      statusWarn: this.#message?.warn ?? this.repos.list.some((repo) => repo.status.warn),
      busyText: op?.text ?? "",
      canCancel: op?.cancel != null,
      hasSelection: item !== null,
    };
    if (!item) return base;
    const caps = item.best?.capabilities ?? item.installed?.capabilities ?? item.entry?.versions[0]?.capabilities ?? [];
    const perms = permissions(caps);
    return {
      ...base,
      dName: item.name,
      dMeta: `${item.author ?? "未知作者"} · ${this.#origin(item)}`,
      dPunchline: item.punchline ?? item.local?.description ?? "",
      dIcon: this.#icon(item),
      actions: this.#actions(item).map((a) => ({ ...a, width: labelWidth(a.text) })),
      banners: this.#banners(item).map((b) => ({ ...b, actionWidth: b.actionText ? labelWidth(b.actionText, 41) : 0 })),
      tab: this.tab,
      doc: this.#doc.key === this.#docKey(item) && this.#doc.ref ? this.#doc.ref : [],
      docNote: this.#doc.key === this.#docKey(item) ? this.#doc.note : "正在加载…",
      versions: this.#versions(item),
      facts: this.#facts(item).map(({ label, text, url }) => ({ label, text, link: url !== null })),
      permissionsTitle: `权限（${perms.length}）`,
      permissions: perms,
      permissionNote: TRUST_NOTE,
      manageNote: item.installed && !item.installed.builtin ? MANAGE_NOTE : "",
    };
  }

  #repoStatus(): string {
    return this.repos.list
      .map((repo) => repo.status.text)
      .filter(Boolean)
      .join(" · ");
  }

  #emptyText(visible: Item[]): string {
    if (visible.length) return "";
    if (this.search || this.tag) return "没有符合条件的插件";
    if (this.filter === 2) return "所有插件都是最新的";
    if (this.filter === 1) return "还没有安装插件";
    if (this.repos.list.some((repo) => repo.refreshing)) return "正在读取仓库…";
    if (this.repos.list.every((repo) => !repo.index)) return "无法连接仓库，也没有缓存。连上网络后点「刷新」。";
    return "仓库里还没有插件";
  }

  #origin(item: Item): string {
    if (item.repo) return item.repo.official ? "官方仓库" : `非官方：${item.repo.label}`;
    return sourceName(item.installed?.source ?? null);
  }

  #row(item: Item) {
    const installedVersion = item.installed?.version ?? "";
    const shown = item.state === "otherSource" || item.state === "available" || item.state === "incompatible" || item.state === "allRevoked"
      ? (item.best ?? item.entry?.versions[0])?.version ?? ""
      : installedVersion;
    const repoTag = item.repo ? (item.repo.official ? "官方" : `非官方：${item.repo.label}`) : sourceName(item.installed?.source ?? null);
    const [badge, tone] = ((): [string, number] => {
      switch (item.state) {
        case "installed":
          return ["已安装", 0];
        case "update":
          return [`可更新 → ${item.best!.version}`, 1];
        case "revoked":
          return ["已撤回", 3];
        case "otherSource":
          return [`已从${sourceName(item.installed?.source ?? null)}安装`, 4];
        case "builtin":
          return ["内置", 4];
        case "incompatible":
          return ["不兼容", 4];
        case "allRevoked":
          return ["已撤回", 3];
        case "local":
          return ["已安装", 0];
        default:
          return ["", 0];
      }
    })();
    return {
      key: item.key,
      name: item.name,
      version: shown,
      meta: `${item.author ?? "未知作者"} · ${repoTag}`,
      punchline: item.punchline ?? "",
      badge,
      tone,
      selected: item.key === this.selected,
    };
  }

  #icon(item: Item): ImageRef {
    let path: string | null = null;
    // A third-party icon on another host than its index is never fetched (it would tell that host who looks).
    if (item.entry?.iconUrl && this.#sameHost(item, item.entry.iconUrl))
      path = this.icons.remote(item.entry.iconUrl, item.repo?.local ?? false, item.entry.iconCheck);
    else if (item.local?.icon && item.installed) path = this.icons.local(join(item.installed.directory, item.local.icon));
    return this.#image(path ?? this.#logo);
  }

  #image(path: string): ImageRef {
    let ref = this.#images.get(path);
    if (!ref) {
      ref = this.plugin.ui.image(path);
      this.#images.set(path, ref);
    }
    return ref;
  }

  #actions(item: Item): { id: string; text: string; enabled: boolean }[] {
    const idle = !this.busy;
    const info = item.installed;
    const list: { id: string; text: string; enabled: boolean }[] = [];
    const newer =
      item.best && info?.version && isVersion(info.version) && compareVersions(item.best.version, info.version) > 0;
    switch (item.state) {
      case "available":
        list.push({ id: "install", text: `安装 ${item.best!.version}`, enabled: idle });
        break;
      case "update":
        list.push({ id: "update", text: `更新到 ${item.best!.version}`, enabled: idle });
        break;
      case "revoked":
        if (newer) list.push({ id: "update", text: `更新到 ${item.best!.version}`, enabled: idle });
        if (info?.state === "disabled") list.push({ id: "enable", text: "仍然启用…", enabled: idle });
        break;
      case "otherSource":
        if (item.best) list.push({ id: "replace", text: "替换…", enabled: idle });
        break;
    }
    if (info && !info.builtin && (item.state === "installed" || item.state === "update" || item.state === "revoked" || item.state === "local")) {
      if (info.previousVersion) list.push({ id: "rollback", text: `回退到 ${info.previousVersion}`, enabled: idle });
      list.push({ id: "uninstall", text: "卸载…", enabled: idle });
    }
    return list;
  }

  #banners(item: Item): { text: string; tone: number; action: string; actionText: string }[] {
    const banners: { text: string; tone: number; action: string; actionText: string }[] = [];
    const add = (text: string, tone: number, action = "", actionText = "") => banners.push({ text, tone, action, actionText });
    const info = item.installed;
    if (item.state === "revoked" && item.revocation && info) {
      const key = revocationKey(item.repo!.url, item.id, info.version ?? "", item.revocation.versions);
      const handled = this.settings.value.revocations[key];
      const tail =
        handled === "accepted"
          ? "你已确认风险并重新启用，商城不会再停用它。"
          : info.state === "disabled"
            ? "商城已停用它。"
            : "";
      add(`此版本已被撤回：${item.revocation.reason}。${tail}`, 3);
    }
    if (item.state === "allRevoked" || item.state === "incompatible") add(item.blocked ?? "", item.state === "allRevoked" ? 3 : 2);
    if (item.repo && !item.repo.official) {
      add("非官方仓库：未经 Vestra Player 审核，插件可能读取你的文件或联网", 2);
      if (item.repo.local) add("这是本机上的测试仓库，没有加密", 2);
    }
    if (item.state === "update" && item.best && info) {
      const added = permissions(item.best.capabilities.filter((c) => !info.capabilities.includes(c)));
      if (added.length) add(`新版本需要新的权限：${added.map((p) => p.text).join("、")}`, 2);
    }
    if (item.state === "otherSource" && info)
      add(`已从${sourceName(info.source)}安装 ${info.version ?? ""}。替换后使用新的空数据，原数据保留`, 0);
    if (item.state === "builtin")
      add(item.repo ? "与播放器内置插件同 id，不能从这里安装" : "内置插件随播放器更新，不能卸载；停用请到「设置 ▶ 插件管理…」", 4);
    if (item.state === "local" && info) {
      if (item.delisted) add("仓库已不再提供这个插件，商城不再为它检查更新（仍会检查撤回）", 4);
      else if (info.source?.kind === "repository") add("它的仓库已从商城移除，商城不再为它检查更新和撤回", 4);
      else add("侧载插件：未经审核，商城不会为它检查更新", 2);
    }
    if (item.development) add("本次运行使用开发加载的版本", 4);
    const doc = this.#doc;
    if (doc.key === this.#docKey(item)) {
      if (doc.offHost) add(`说明位于其它网站（${doc.offHost}），没有自动加载`, 2, "showImages", "加载");
      if (doc.blocked > 0) add(`已阻止 ${doc.blocked} 张远程图片（非官方来源）`, 2, "showImages", "显示图片");
      if (doc.failed) add(doc.note, 2, "reloadDoc", "重试");
    }
    return banners;
  }

  #versions(item: Item) {
    if (!item.entry) {
      const info = item.installed;
      return info?.version
        ? [{ version: info.version, date: "", minimum: "", status: "当前", tone: 0, canInstall: false }]
        : [];
    }
    const revoked = item.repo?.index?.revoked ?? [];
    const sameSource = item.state === "installed" || item.state === "update" || item.state === "revoked";
    return item.entry.versions.map((v) => {
      const check = checkVersion(item.id, v, revoked, this.plugin.app.version);
      const current = sameSource && item.installed?.version === v.version;
      const status = current
        ? check.ok
          ? "当前"
          : `当前，${check.text}`
        : !check.ok
          ? check.why === "revoked"
            ? check.text
            : `不兼容：${check.text}`
          : v === item.best
            ? "最新"
            : "";
      return {
        version: v.version,
        date: day(v.published),
        minimum: v.minPlayerVersion ?? "—",
        status,
        tone: check.ok ? 0 : check.why === "revoked" ? 3 : 4,
        canInstall: check.ok && !current && item.state !== "builtin" && !this.busy,
      };
    });
  }

  #facts(item: Item | null): { label: string; text: string; url: string | null }[] {
    if (!item) return [];
    const facts: { label: string; text: string; url: string | null }[] = [];
    const latest = item.best ?? item.entry?.versions[0];
    if (latest) facts.push({ label: "版本", text: latest.version, url: null });
    if (item.installed) facts.push({ label: "已安装", text: item.installed.version ?? "—", url: null });
    if (latest) facts.push({ label: "更新", text: day(latest.published), url: null });
    facts.push({ label: "来源", text: item.repo ? item.repo.label : sourceName(item.installed?.source ?? null), url: null });
    facts.push({ label: "id", text: item.id, url: null });
    const license = item.entry?.license ?? item.local?.license;
    if (license) facts.push({ label: "许可证", text: license, url: null });
    const source = item.entry?.sourceUrl ?? item.local?.sourceUrl;
    if (source) facts.push({ label: "源码", text: "打开 ↗", url: source });
    return facts;
  }

  async #act(action: string) {
    const item = this.selectedItem();
    if (!item) return;
    switch (action) {
      case "install":
      case "update":
      case "replace":
        if (item.best) await installer.installFromRepository(this, item, item.best);
        return;
      case "rollback":
        return installer.rollback(this, item);
      case "uninstall":
        return installer.uninstall(this, item);
      case "enable":
        return this.revocations.enable(item);
    }
  }

  #bannerAct(action: string) {
    const item = this.selectedItem();
    if (!item) return;
    if (action === "showImages") {
      this.#remoteShown.add(item.key);
      void this.#loadDoc();
    } else if (action === "reloadDoc") {
      this.#doc = { ...this.#doc, key: "" };
      void this.#loadDoc();
    }
  }

  async #openLink(url: string) {
    if (/^https?:\/\//i.test(url)) {
      try {
        await this.plugin.system.openUrl(url);
      } catch (error) {
        this.report(`无法打开链接：${(error as Error).message}`, true);
      }
    } else if (!url.startsWith("#")) this.report("不支持打开这种链接", true);
  }

  async #fromFile() {
    let path: string | null;
    try {
      path = await this.plugin.dialogs.openFile({ title: "选择插件包（.vxp）" });
    } catch (error) {
      if ((error as { code?: string }).code === "busy") this.report("另一个对话框正在打开，请先关闭它", true);
      return;
    }
    if (path) await installer.installFile(this, path);
  }

  // ------------------------------------------------------------ README / CHANGELOG

  /** Official resources, or third-party ones on the index's own host (design §6.1). */
  #sameHost(item: Item, url: string): boolean {
    if (!item.repo || item.repo.official) return true;
    try {
      return new URL(url).origin === new URL(item.repo.url).origin;
    } catch {
      return false;
    }
  }

  #remoteImages(item: Item): boolean {
    return item.repo?.official === true || this.#remoteShown.has(item.key);
  }

  #docKey(item: Item): string {
    const version = item.best?.version ?? item.installed?.version ?? "";
    return `${item.key}\n${version}\n${this.tab}\n${this.#remoteImages(item)}`;
  }

  async #loadDoc() {
    const item = this.selectedItem();
    if (!item || this.tab === 2 || this.#disposed) return;
    const key = this.#docKey(item);
    if (this.#doc.key === key) return;
    this.#docAbort?.abort();
    const abort = (this.#docAbort = new AbortController());
    const changelog = this.tab === 1;
    const previous = this.#doc;
    this.#doc = { key, ref: null, note: changelog ? "正在加载更新日志…" : "正在加载说明…", blocked: 0, failed: false, scope: previous.scope };
    this.changed();
    const empty = changelog ? "作者没有提供更新日志" : "作者没有提供说明";
    let text: string | null = null;
    let base: string | undefined;
    try {
      if (item.entry) {
        const url = changelog ? item.entry.changelogUrl : item.entry.readmeUrl;
        const check = changelog ? item.entry.changelogCheck : item.entry.readmeCheck;
        if (url && !this.#sameHost(item, url) && !this.#remoteShown.has(item.key)) {
          this.#doc = { ...this.#doc, note: `${changelog ? "更新日志" : "说明"}位于其它网站，未自动加载`, offHost: hostOf(url) };
          this.changed();
          return;
        }
        if (url) {
          base = url;
          text = this.#texts.get(url) ?? null;
          if (text === null) {
            if (check && check.size > README_LIMIT) throw new Error("文件超过 256 KB");
            // With the index's size and sha256 the text must match exactly (else nothing is shown or cached).
            const got = await get(this.deps.fetch, url, {
              maxBytes: check ? check.size : README_LIMIT,
              ...(check ? { exactSize: check.size, sha256: check.sha256 } : {}),
              timeoutMs: 10_000,
              signal: abort.signal,
              headers: { "User-Agent": this.deps.userAgent },
              allowLocalHttp: item.repo?.local ?? false,
            });
            text = got.bytes!.toString("utf8");
            this.#texts.set(url, text);
          }
        }
      } else if (item.installed && item.local) {
        const name = changelog ? item.local.changelog : item.local.readme;
        if (name) text = readLimited(join(item.installed.directory, name));
      }
    } catch (error) {
      if (abort.signal.aborted || this.#doc.key !== key) return;
      this.#doc = { ...this.#doc, note: `无法加载${changelog ? "更新日志" : "说明"}：${(error as Error).message}`, failed: true };
      this.changed();
      return;
    }
    if (this.#doc.key !== key) return;
    if (text === null || !text.trim()) {
      this.#doc = { ...this.#doc, note: empty };
      this.changed();
      return;
    }
    const remoteImages = this.#remoteImages(item);
    const blocked = remoteImages ? 0 : new Set([...text.matchAll(REMOTE_IMAGE)].map((m) => m[1])).size;
    const scope = this.plugin.scope();
    try {
      const ref = await scope.ui.markdown(text, { ...(base ? { base } : {}), remoteImages });
      if (this.#doc.key !== key) {
        await scope.dispose();
        return;
      }
      this.#doc = { key, ref, note: "", blocked, failed: false, scope };
    } catch (error) {
      await scope.dispose();
      if (this.#doc.key !== key) return;
      this.#doc = { ...this.#doc, note: `无法显示${changelog ? "更新日志" : "说明"}：${(error as Error).message}`, failed: true };
    }
    this.changed();
    // The old document's scope goes once the new one is in the window.
    if (previous.scope && previous.scope !== this.#doc.scope) setTimeout(() => void previous.scope?.dispose(), 1000);
  }

  /** Display fields of an installed plugin from its manifest (cached per directory). */
  #display(info: PluginInfo): Display {
    const cached = this.#displays.get(`${info.directory}\n${info.version}`);
    if (cached) return cached;
    let manifest: Record<string, unknown> = {};
    try {
      manifest = JSON.parse(readFileSync(join(info.directory, "manifest.json"), "utf8")) as Record<string, unknown>;
    } catch {
      // Not readable: show the id.
    }
    const file = (value: unknown, fallback: string | null) =>
      typeof value === "string" && !value.includes("..") ? value : fallback;
    const display: Display = {
      name: clean(manifest.name, 64) ?? info.id,
      author: clean(manifest.author, 64),
      punchline: clean(manifest.punchline, 120),
      description: clean(manifest.description, 2000, true),
      tags: Array.isArray(manifest.tags)
        ? manifest.tags.map((t) => clean(t, 24)).filter((t): t is string => t !== null).slice(0, 8)
        : [],
      license: clean(manifest.license, 64),
      sourceUrl: typeof manifest.sourceUrl === "string" && /^https:\/\//.test(manifest.sourceUrl) ? manifest.sourceUrl : null,
      icon: file(manifest.icon, null),
      readme: file(manifest.readme, "README.md"),
      changelog: file(manifest.changelog, "CHANGELOG.md"),
    };
    this.#displays.set(`${info.directory}\n${info.version}`, display);
    return display;
  }
}

function readLimited(path: string): string | null {
  try {
    const bytes = readFileSync(path);
    return bytes.length > README_LIMIT ? null : bytes.toString("utf8");
  } catch {
    return null;
  }
}

function ownVersion(): string {
  try {
    const manifest = JSON.parse(readFileSync(new URL("./manifest.json", import.meta.url), "utf8")) as { version?: string };
    return manifest.version ?? "0.0.0";
  } catch {
    return "0.0.0";
  }
}
