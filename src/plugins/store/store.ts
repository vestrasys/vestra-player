import { readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type {
  ImageRef,
  TextValue,
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
import { day, hostOf, permissions, reasonOf, sourceName } from "./labels.ts";
import { get, NetError } from "./net.ts";
import { Revocations, revocationKey } from "./revocations.ts";
import { Repos, type Deps } from "./repos.ts";
import { compareVersions, isVersion } from "./semver.ts";
import { SettingsPage } from "./settings-page.ts";
import { argOf, joined, list, msg, raw, textOf, type Arg, type Label, type Msg } from "./text.ts";

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
const REMOTE_IMAGE = /!\[[^\]]*\]\(\s*<?(https?:\/\/[^)\s>]+)/g;

/** A confirmation: `message` and `detail` share one list of arguments (`detail` numbers its placeholders after the message's). */
export interface Question {
  message: Msg;
  detail?: Msg;
  confirm: Label;
  cancel?: Label;
  destructive?: boolean;
  checkbox?: Label;
}

type Banner = { text: Msg | null; tone: number; action: string; actionText: Msg | null };

type Doc = {
  key: string;
  ref: MarkdownRef | null;
  note: Msg | null;
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
  #message: { text: Msg; warn: boolean } | null = null;
  #operation: { text: Msg; cancel: (() => void) | null } | null = null;
  #doc: Doc = { key: "", ref: null, note: null, blocked: 0, failed: false, scope: null };
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

  /** "Plugin Store…", with the number of updates in the title (design §1.2). */
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
        path: [count > 0 ? this.plugin.i18n.text("Plugin Store ({} updates)…", [count]) : "Plugin Store…"],
        order: MENU_ORDER,
        group: "windows",
        run: () => this.open(),
      });
      if (afterRefresh && count > 0 && before <= 0 && !this.#announced) {
        this.#announced = true;
        this.osd(msg("Plugin Store: plugin updates available ({})", count));
      }
    });
    return this.#menuChain;
  }

  // ------------------------------------------------------------ operations

  /** One download or install at a time. */
  begin(text: Msg, cancel: (() => void) | null = null): boolean {
    if (this.#operation) {
      this.report(msg("Another install is in progress; wait for it to finish"), true);
      return false;
    }
    this.#operation = { text, cancel };
    this.#message = null;
    this.changed();
    return true;
  }

  progress(text: Msg, cancel?: (() => void) | null) {
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
  report(text: Msg, warn = false) {
    this.#message = { text, warn };
    this.osd(text);
    this.changed();
  }

  osd(text: Msg) {
    // Arguments cut so the shown text stays within the OSD's 200 characters.
    void this.plugin.osd.show(text.text, text.args.map((arg) => argOf(this.plugin.i18n, arg, 80))).catch(() => {});
  }

  /** A `Msg` as window text. */
  shown(text: Arg | null): TextValue {
    return textOf(this.plugin.i18n, text);
  }

  /** `dialogs.confirm` with the limits applied; `null` when no dialog could be shown. */
  async ask(question: Question): Promise<{ confirmed: boolean; checked: boolean } | null> {
    this.lastDialogError = null;
    const { message, detail, confirm, cancel, destructive, checkbox } = question;
    const args = [...message.args, ...(detail?.args ?? [])].map((arg) => argOf(this.plugin.i18n, arg, 1200));
    try {
      return await this.plugin.dialogs.confirm({
        message: message.text,
        ...(detail === undefined ? {} : { detail: detail.text }),
        confirm,
        ...(cancel === undefined ? {} : { cancel }),
        ...(destructive === undefined ? {} : { destructive }),
        ...(checkbox === undefined ? {} : { checkbox }),
        ...(args.length ? { args } : {}),
      });
    } catch (error) {
      const code = (error as { code?: string }).code;
      this.lastDialogError = code ?? "failed";
      if (code === "unsupported") this.report(msg("This system does not support confirmation dialogs; cannot continue"), true);
      else if (code === "busy") this.report(msg("Another dialog is open; close it first"), true);
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
    const op = this.#operation;
    // One message, or a text per repository.
    const status = this.#message
      ? [this.#message.text]
      : this.repos.list.flatMap((repo) => (repo.status.text ? [repo.status.text] : []));
    const shown = (text: Msg | null) => this.shown(text);
    const base = {
      filter: this.filter,
      updates: updateCount(this.items),
      // The first entry is "Tags: All" (the window's words).
      tagModel: ["", ...tags],
      tagIndex: this.tag ? Math.max(0, tags.findIndex((t) => t.toLowerCase() === this.tag!.toLowerCase()) + 1) : 0,
      sortIndex: this.sort,
      rows: visible.map((each) => this.#row(each)),
      emptyText: shown(this.#emptyText(visible)),
      linkOpen: this.linkOpen,
      status: this.shown(joined(status, "{} · {}")),
      statusWarn: this.#message?.warn ?? this.repos.list.some((repo) => repo.status.warn),
      busyText: shown(op?.text ?? null),
      canCancel: op?.cancel != null,
      hasSelection: item !== null,
    };
    if (!item) return base;
    const caps = item.best?.capabilities ?? item.installed?.capabilities ?? item.entry?.versions[0]?.capabilities ?? [];
    return {
      ...base,
      dName: item.name,
      dMeta: shown(msg("{} · {}", item.author ?? msg("Unknown author"), this.#origin(item))),
      dPunchline: item.punchline ?? item.local?.description ?? "",
      dIcon: this.#icon(item),
      actions: this.#actions(item).map((a) => ({ ...a, text: shown(a.text) })),
      banners: this.#banners(item).map((b) => ({ ...b, text: shown(b.text), actionText: shown(b.actionText) })),
      tab: this.tab,
      doc: this.#doc.key === this.#docKey(item) && this.#doc.ref ? this.#doc.ref : [],
      docNote: shown(this.#doc.key === this.#docKey(item) ? this.#doc.note : msg("Loading…")),
      versions: this.#versions(item),
      facts: this.#facts(item).map(({ label, text, url }) => ({ label: shown(label), text: shown(text), link: url !== null })),
      permissions: permissions(caps).map(({ text, risky }) => ({ text: shown(text), risky })),
      manage: item.installed !== null && !item.installed.builtin,
    };
  }

  #emptyText(visible: Item[]): Msg | null {
    if (visible.length) return null;
    if (this.search || this.tag) return msg("No plugins match");
    if (this.filter === 2) return msg("All plugins are up to date");
    if (this.filter === 1) return msg("No plugins installed yet");
    if (this.repos.list.some((repo) => repo.refreshing)) return msg("Reading repositories…");
    if (this.repos.list.every((repo) => !repo.index))
      return msg("Can't connect to the repositories and there is no cached copy. Click “Refresh” once you are online.");
    return msg("The repositories have no plugins yet");
  }

  #origin(item: Item): Msg {
    if (item.repo) return item.repo.origin;
    return sourceName(item.installed?.source ?? null);
  }

  #row(item: Item) {
    const installedVersion = item.installed?.version ?? "";
    const shown = item.state === "otherSource" || item.state === "available" || item.state === "incompatible" || item.state === "allRevoked" || item.state === "reserved"
      ? (item.best ?? item.entry?.versions[0])?.version ?? ""
      : installedVersion;
    const [badge, tone] = ((): [Msg | null, number] => {
      switch (item.state) {
        case "installed":
          return [msg("Installed"), 0];
        case "update":
          return [msg("Update → {}", item.best!.version), 1];
        case "revoked":
          return [msg("Revoked"), 3];
        case "otherSource":
          return [msg("Installed from {}", sourceName(item.installed?.source ?? null)), 4];
        case "builtin":
          return [msg("Built-in"), 4];
        case "incompatible":
          return [msg("Incompatible"), 4];
        case "reserved":
          return [msg("Cannot be installed"), 3];
        case "allRevoked":
          return [msg("Revoked"), 3];
        case "local":
          return [msg("Installed"), 0];
        default:
          return [null, 0];
      }
    })();
    return {
      key: item.key,
      name: item.name,
      version: shown,
      meta: this.shown(msg("{} · {}", item.author ?? msg("Unknown author"), this.#origin(item))),
      punchline: item.punchline ?? "",
      badge: this.shown(badge),
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

  #actions(item: Item): { id: string; text: Msg; enabled: boolean }[] {
    const idle = !this.busy;
    const info = item.installed;
    const list: { id: string; text: Msg; enabled: boolean }[] = [];
    const newer =
      item.best && info?.version && isVersion(info.version) && compareVersions(item.best.version, info.version) > 0;
    switch (item.state) {
      case "available":
        list.push({ id: "install", text: msg("Install {}", item.best!.version), enabled: idle });
        break;
      case "update":
        list.push({ id: "update", text: msg("Update to {}", item.best!.version), enabled: idle });
        break;
      case "revoked":
        if (newer) list.push({ id: "update", text: msg("Update to {}", item.best!.version), enabled: idle });
        if (info?.state === "disabled") list.push({ id: "enable", text: msg("Enable Anyway…"), enabled: idle });
        break;
      case "otherSource":
        if (item.best) list.push({ id: "replace", text: msg("Replace…"), enabled: idle });
        break;
    }
    if (info && !info.builtin && (item.state === "installed" || item.state === "update" || item.state === "revoked" || item.state === "local")) {
      if (info.previousVersion)
        list.push({
          id: "rollback",
          text:
            installer.rollbackVerb(info.version, info.previousVersion) === "restore"
              ? msg("Restore to {}", info.previousVersion)
              : msg("Roll Back to {}", info.previousVersion),
          enabled: idle,
        });
      list.push({ id: "uninstall", text: msg("Uninstall…"), enabled: idle });
    }
    return list;
  }

  #banners(item: Item): Banner[] {
    const banners: Banner[] = [];
    const add = (text: Msg | null, tone: number, action = "", actionText: Msg | null = null) =>
      banners.push({ text, tone, action, actionText });
    const info = item.installed;
    if (item.state === "revoked" && item.revocation && info) {
      const key = revocationKey(item.repo!.url, item.id, info.version ?? "", item.revocation.versions);
      const handled = this.settings.value.revocations[key];
      const reason = item.revocation.reason;
      add(
        handled === "accepted"
          ? msg(
              "This version has been revoked: {}. You confirmed the risk and enabled it again; the store will not disable it again.",
              reason,
            )
          : info.state === "disabled"
            ? msg("This version has been revoked: {}. The store has disabled it.", reason)
            : msg("This version has been revoked: {}.", reason),
        3,
      );
    }
    if (item.state === "allRevoked" || item.state === "incompatible" || item.state === "reserved")
      add(item.blocked, item.state === "incompatible" ? 2 : 3);
    if (item.repo && !item.repo.official) {
      add(msg("Unofficial repository: not reviewed by Vestra Player; plugins may read your files or use the network"), 2);
      if (item.repo.local) add(msg("This is a test repository on this machine, without encryption"), 2);
    }
    if (item.state === "update" && item.best && info) {
      const added = permissions(item.best.capabilities.filter((c) => !info.capabilities.includes(c)));
      if (added.length) add(msg("The new version needs new permissions: {}", list(added.map((p) => p.text))), 2);
    }
    if (item.state === "otherSource" && info)
      add(
        msg(
          "Installed {1} from {0}. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data; the current data is kept",
          sourceName(info.source),
          info.version ?? "",
        ),
        0,
      );
    if (item.state === "builtin")
      add(
        item.repo
          ? msg("Same id as a built-in plugin of the player; cannot be installed from here")
          : msg(
              "Built-in plugins are updated with the player and cannot be uninstalled; to disable one, use “Settings ▸ Plugin Manager…”",
            ),
        4,
      );
    if (item.state === "local" && info) {
      if (item.delisted)
        add(
          msg(
            "The repository no longer offers this plugin; the store no longer checks it for updates (it still checks for revocations)",
          ),
          4,
        );
      else if (info.source?.kind === "repository")
        add(msg("Its repository was removed from the store; the store no longer checks it for updates or revocations"), 4);
      else add(msg("Sideloaded plugin: not reviewed; the store does not check it for updates"), 2);
    }
    if (item.development) add(msg("This run uses the development-loaded version"), 4);
    const doc = this.#doc;
    if (doc.key === this.#docKey(item)) {
      if (doc.offHost)
        add(msg("The description is on another website ({}); not loaded automatically", doc.offHost), 2, "showImages", msg("Load"));
      if (doc.blocked > 0)
        add(msg("Remote images blocked (unofficial source): {}", doc.blocked), 2, "showImages", msg("Show Images"));
      if (doc.failed) add(doc.note, 2, "reloadDoc", msg("Retry"));
    }
    return banners;
  }

  #versions(item: Item) {
    if (!item.entry) {
      const info = item.installed;
      return info?.version
        ? [{ version: info.version, date: "", minimum: "", status: this.shown(msg("Current")), tone: 0, canInstall: false }]
        : [];
    }
    const revoked = item.repo?.index?.revoked ?? [];
    const sameSource = item.state === "installed" || item.state === "update" || item.state === "revoked";
    return item.entry.versions.map((v) => {
      const check = checkVersion(item.id, v, revoked, this.plugin.app.version);
      const current = sameSource && item.installed?.version === v.version;
      const status = current
        ? check.ok
          ? msg("Current")
          : msg("Current, {}", check.text)
        : !check.ok
          ? check.why === "revoked"
            ? check.text
            : msg("Incompatible: {}", check.text)
          : v === item.best
            ? msg("Latest")
            : null;
      return {
        version: v.version,
        date: day(v.published),
        minimum: v.minPlayerVersion ?? "—",
        status: this.shown(status),
        tone: check.ok ? 0 : check.why === "revoked" ? 3 : 4,
        canInstall: check.ok && !current && item.state !== "builtin" && item.state !== "reserved" && !this.busy,
      };
    });
  }

  #facts(item: Item | null): { label: Msg; text: Msg; url: string | null }[] {
    if (!item) return [];
    const facts: { label: Msg; text: Msg; url: string | null }[] = [];
    const latest = item.best ?? item.entry?.versions[0];
    if (latest) facts.push({ label: msg("Version"), text: raw(latest.version), url: null });
    if (item.installed) facts.push({ label: msg("Installed"), text: raw(item.installed.version ?? "—"), url: null });
    if (latest) facts.push({ label: msg("Updated"), text: raw(day(latest.published)), url: null });
    facts.push({ label: msg("Source"), text: item.repo ? item.repo.label : sourceName(item.installed?.source ?? null), url: null });
    facts.push({ label: raw("id"), text: raw(item.id), url: null });
    const license = item.entry?.license ?? item.local?.license;
    if (license) facts.push({ label: msg("License"), text: raw(license), url: null });
    const source = item.entry?.sourceUrl ?? item.local?.sourceUrl;
    if (source) facts.push({ label: msg("Code"), text: msg("Open ↗"), url: source });
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
        this.report(msg("Couldn't open the link: {}", raw((error as Error).message)), true);
      }
    } else if (!url.startsWith("#")) this.report(msg("This kind of link cannot be opened"), true);
  }

  async #fromFile() {
    let path: string | null;
    try {
      path = await this.plugin.dialogs.openFile({ title: "Choose a plugin package (.vxp)" });
    } catch (error) {
      if ((error as { code?: string }).code === "busy") this.report(msg("Another dialog is open; close it first"), true);
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
    this.#doc = {
      key,
      ref: null,
      note: changelog ? msg("Loading the changelog…") : msg("Loading the description…"),
      blocked: 0,
      failed: false,
      scope: previous.scope,
    };
    this.changed();
    const empty = changelog ? msg("The author provided no changelog") : msg("The author provided no description");
    let text: string | null = null;
    let base: string | undefined;
    try {
      if (item.entry) {
        const url = changelog ? item.entry.changelogUrl : item.entry.readmeUrl;
        const check = changelog ? item.entry.changelogCheck : item.entry.readmeCheck;
        if (url && !this.#sameHost(item, url) && !this.#remoteShown.has(item.key)) {
          this.#doc = {
            ...this.#doc,
            note: changelog
              ? msg("The changelog is on another website; not loaded automatically")
              : msg("The description is on another website; not loaded automatically"),
            offHost: hostOf(url),
          };
          this.changed();
          return;
        }
        if (url) {
          base = url;
          text = this.#texts.get(url) ?? null;
          if (text === null) {
            if (check && check.size > README_LIMIT) throw new NetError("tooLarge", msg("File exceeds 256 KB"));
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
      const why = reasonOf(error);
      this.#doc = {
        ...this.#doc,
        note: changelog ? msg("Couldn't load the changelog: {}", why) : msg("Couldn't load the description: {}", why),
        failed: true,
      };
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
      this.#doc = { key, ref, note: null, blocked, failed: false, scope };
    } catch (error) {
      await scope.dispose();
      if (this.#doc.key !== key) return;
      const why = raw((error as Error).message);
      this.#doc = {
        ...this.#doc,
        note: changelog ? msg("Couldn't show the changelog: {}", why) : msg("Couldn't show the description: {}", why),
        failed: true,
      };
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
