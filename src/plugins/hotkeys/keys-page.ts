import { renameSync, writeFileSync } from "node:fs";
import type { BindingInfo, BindingRef, JsonStore, Plugin } from "@vestra-player/sdk";
import { exportCsv, exportText } from "./export.ts";
import type { IgnoreDefaults } from "./ignore.ts";
import {
  commandText,
  fromSlintKey,
  isMouse,
  label,
  refKey,
  sameKey,
  sourceText,
} from "./keys.ts";
import { type BindingsModel, failureText } from "./model.ts";
import { jsonStore, PREFS, type Prefs, readPrefs } from "./store.ts";

/** Status filter choices (J01). */
export const STATUS = ["全部", "已修改", "有冲突", "未绑定"];
const ORPHANS = "*orphans";
/** J01: at most this many bindings per command. */
export const MAX_BINDINGS = 8;

/** One row of the page's list (命令 | 快捷键 | 状态). */
type Row = {
  key: string;
  command: string;
  keys: string;
  status: string;
  tone: number;
  selected: boolean;
  tip: string;
};
/** One line of the table, for rows, the edit area and the exported list. */
export type Line = Omit<Row, "selected"> & {
  scope: string;
  source: string;
  info: BindingInfo | null;
  ref: BindingRef;
  inputs: string[];
};

/** The J01 page in the options window (category 快捷键与鼠标). */
export const KEYS_PAGE = "hotkeys.keyboard";

/** 已修改 / 冲突 / 已忽略 / 默认 for one command (J01, J03). */
export function lineOf(info: BindingInfo, ignored: boolean, model: BindingsModel): Line {
  const keyboard = info.labels.filter((_, i) => !isMouse(info.current[i] ?? ""));
  let keys = keyboard.join(", ");
  let status = "默认";
  let tone = 0;
  let tip = "";
  if (ignored) {
    status = "已忽略";
    tone = 3;
    const defaults = info.defaults.filter((input) => !isMouse(input)).map(label);
    keys = defaults.length ? `（${defaults.join(", ")}）` : "";
    tip = "默认快捷键已忽略（设置 ▶ 忽略默认快捷键）";
  } else if (info.shadowed.length) {
    status = "冲突";
    tone = 2;
    tip = info.shadowed
      .map((s) => `${label(s.input)} 被「${model.text(s.by)}」占用，未生效`)
      .join("；");
  } else if (info.overridden) {
    status = "已修改";
    tone = 1;
  }
  return {
    key: refKey(info),
    ref: { plugin: info.plugin, command: info.command },
    info,
    inputs: info.current,
    command: commandText(info),
    keys,
    scope: info.scope === "panel" ? "面板内" : "全局",
    source: sourceText(info.plugin),
    status,
    tone,
    tip,
  };
}

/**
 * 快捷键 (J01), a page of the options window: every command of the binding table with its effective
 * keys, filtered and sorted; the selected command's bindings as tags that
 * can be removed, a capture box to add one (conflicts ask 替换 / 取消),
 * 恢复默认 / 全部解绑, and 恢复全部默认… / 导出列表… / 忽略默认快捷键.
 * The list follows `bindings.onChange`; the selection stays by command.
 */
export class KeysPage {
  readonly #plugin: Plugin;
  readonly #model: BindingsModel;
  readonly #ignore: IgnoreDefaults;
  readonly #osd: (text: string) => void;
  readonly #page;
  readonly #prefs: JsonStore<Prefs>;
  #search = "";
  #selected: string | null = null;
  #capturing = false;
  #pending: { input: string; owners: BindingInfo[]; target: BindingRef } | null = null;
  #message = { text: "", warn: false };
  #rows: { key: string; rows: Row[]; lines: Line[] } = { key: "", rows: [], lines: [] };

  constructor(plugin: Plugin, model: BindingsModel, ignore: IgnoreDefaults, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#model = model;
    this.#ignore = ignore;
    this.#osd = osd;
    this.#prefs = jsonStore<Prefs>(plugin, "hotkeys", PREFS);
    const page = (this.#page = plugin.settings.page("ui/keys.slint", {
      id: KEYS_PAGE,
      category: "input",
      title: "快捷键",
      order: 10,
    }));
    page.on("search", (text: string) => {
      this.#search = text;
    });
    page.on("statusFilter", (index: number) => this.#savePrefs({ status: index }));
    page.on("sourceFilter", (index: number) => this.#savePrefs({ source: this.#sources()[index]?.value ?? "" }));
    page.on("sortBy", (column: "command" | "keys") => {
      const prefs = this.#prefsValue();
      this.#savePrefs({ sort: column, descending: prefs.sort === column ? !prefs.descending : false });
    });
    page.on("select", (key: string) => this.#select(key));
    page.on("removeTag", (input: string) => this.#removeTag(input));
    page.on("addKey", () => this.#startCapture());
    page.on("key", (text: string, control: boolean, alt: boolean, shift: boolean, meta: boolean) =>
      this.#key({ text, control, alt, shift, meta }),
    );
    page.on("bindEscape", () => this.#captured("Escape"));
    page.on("cancelCapture", () => {
      this.#capturing = false;
    });
    page.on("resetOne", () => this.#resetOne());
    page.on("unbindAll", () => this.#unbindAll());
    page.on("replace", () => this.#replace());
    page.on("cancelReplace", () => {
      this.#pending = null;
      this.#say("");
    });
    page.on("resetAll", () => this.#resetAll());
    page.on("export", () => this.#export());
    page.on("toggleIgnore", () => ignore.toggle());
    page.bind(() => this.#props());
    model.onChange(() => {
      // A command that went away (plugin stopped) drops the selection.
      if (this.#selected && !this.#selectedLine()) this.#selected = null;
      if (!this.#selected) {
        this.#capturing = false;
        this.#pending = null;
      }
    });
  }

  /** 设置 ▶ 快捷键…: the options window on this page, with a fresh list. */
  async show() {
    await this.#model.reload().catch((error) => this.#say(failureText(error), true));
    await this.#plugin.settings.show(KEYS_PAGE);
  }

  #prefsValue(): Prefs {
    return readPrefs(this.#prefs.value);
  }
  #savePrefs(patch: Partial<Prefs>) {
    this.#prefs.save({ ...this.#prefsValue(), ...patch });
  }

  /** Source filter: all, 宿主, each plugin with commands, 未加载的插件. */
  #sources(): { title: string; value: string }[] {
    const plugins = [...new Set(this.#model.commands.map((c) => c.plugin))]
      .filter((id) => id !== "host")
      .sort();
    return [
      { title: "全部来源", value: "" },
      { title: "宿主", value: "host" },
      ...plugins.map((id) => ({ title: id, value: id })),
      { title: "未加载的插件", value: ORPHANS },
    ];
  }

  /** All lines (unfiltered), commands then orphans. */
  lines(): Line[] {
    const lines = this.#model.commands.map((info) => lineOf(info, this.#ignore.ignores(info), this.#model));
    const orphans = this.#model.orphans.map(
      (o): Line => ({
        key: `${ORPHANS}/${refKey(o.ref)}`,
        ref: o.ref,
        info: null,
        inputs: o.inputs,
        command: refKey(o.ref),
        keys: o.inputs.filter((i) => !isMouse(i)).map(label).join(", "),
        scope: "—",
        source: sourceText(o.ref.plugin),
        status: "未加载",
        tone: 3,
        tip: "插件未加载；它回来后这条修改照常生效",
      }),
    );
    return [...lines, ...orphans];
  }

  #view(): { rows: Row[]; lines: Line[] } {
    const prefs = this.#prefsValue();
    const key = [
      this.#model.version,
      this.#search,
      prefs.status,
      prefs.source,
      prefs.sort,
      prefs.descending,
      this.#selected,
      this.#ignore.enabled,
      JSON.stringify(this.#ignoreKey()),
    ].join("\u0001");
    if (key === this.#rows.key) return this.#rows;
    const search = this.#search.trim().toLowerCase();
    const lines = this.lines()
      .filter((line) => (prefs.source === ORPHANS ? !line.info : !!line.info && (!prefs.source || line.ref.plugin === prefs.source)))
      .filter((line) => {
        if (prefs.status === 1) return line.status === "已修改" || line.status === "已忽略";
        if (prefs.status === 2) return line.status === "冲突";
        if (prefs.status === 3) return line.inputs.length === 0;
        return true;
      })
      .filter(
        (line) =>
          !search ||
          line.command.toLowerCase().includes(search) ||
          line.keys.toLowerCase().includes(search) ||
          line.inputs.some((input) => input.toLowerCase().includes(search)),
      );
    const by = prefs.sort === "keys" ? (line: Line) => line.keys : (line: Line) => line.command;
    lines.sort((a, b) => {
      // Unbound commands go last when sorting by keys.
      if (prefs.sort === "keys" && !a.keys !== !b.keys) return a.keys ? -1 : 1;
      return by(a).localeCompare(by(b), "zh-Hans-CN") || a.command.localeCompare(b.command, "zh-Hans-CN");
    });
    if (prefs.descending) lines.reverse();
    const rows = lines.map(({ key, command, keys, status, tone, tip }) => ({
      key,
      command,
      keys,
      status,
      tone,
      tip,
      selected: key === this.#selected,
    }));
    this.#rows = { key, rows, lines };
    return this.#rows;
  }
  #ignoreKey() {
    return this.#model.commands.filter((c) => this.#ignore.ignores(c)).map(refKey);
  }

  #selectedLine(): Line | undefined {
    if (!this.#selected) return undefined;
    return this.lines().find((line) => line.key === this.#selected);
  }

  #props() {
    const prefs = this.#prefsValue();
    const sources = this.#sources();
    const { rows } = this.#view();
    const line = this.#selectedLine();
    const info = line?.info ?? null;
    return {
      rows,
      emptyText: this.#model.commands.length === 0 ? "正在读取…" : "没有符合条件的命令",
      statusModel: STATUS,
      statusIndex: prefs.status,
      sourceModel: sources.map((s) => s.title),
      sourceIndex: Math.max(
        0,
        sources.findIndex((s) => s.value === prefs.source),
      ),
      sort: prefs.sort,
      descending: prefs.descending,
      hasSelection: !!line,
      editTitle: line
        ? info
          ? `${line.command}（${line.source}${info.scope === "panel" ? " · 面板内" : ""}）`
          : `${line.command}（未加载）`
        : "",
      tags: info ? info.current.map((input, i) => ({ input, label: isMouse(input) ? label(input) : (info.labels[i] ?? label(input)) })) : [],
      canAdd: !!info,
      canReset: !!line && (!info || info.overridden),
      resetText: info || !line ? "恢复默认" : "删除此条",
      canUnbind: !!info && info.current.length > 0,
      capturing: this.#capturing,
      pending: !!this.#pending,
      pendingText: this.#pendingText(),
      message: this.#message.text,
      messageWarn: this.#message.warn,
      ignore: this.#ignore.enabled,
    };
  }

  #say(text: string, warn = false) {
    this.#message = { text, warn };
  }

  #select(key: string) {
    if (key === this.#selected) return;
    this.#selected = key;
    this.#capturing = false;
    this.#pending = null;
    const line = this.#selectedLine();
    this.#say(line?.tip && line.tone === 2 ? line.tip : "");
  }

  #startCapture() {
    if (!this.#selectedLine()?.info) return;
    this.#pending = null;
    this.#capturing = true;
    this.#say("按 Esc 取消；要绑定 Esc 本身，点旁边的「Esc」");
  }

  #key(event: Parameters<typeof fromSlintKey>[0]) {
    if (!this.#capturing) return;
    const result = fromSlintKey(event);
    if (!result) return; // a modifier alone: keep waiting
    if ("cancel" in result) {
      this.#capturing = false;
      this.#say("");
      return;
    }
    if ("error" in result) {
      this.#capturing = false;
      this.#say(result.error, true);
      return;
    }
    return this.#captured(result.input);
  }

  /** A key was captured for the selected command. */
  async #captured(input: string) {
    this.#capturing = false;
    const info = this.#selectedLine()?.info;
    if (!info) return;
    if (info.current.includes(input)) return this.#say(`${label(input)} 已经是这条命令的快捷键`);
    if (info.current.length >= MAX_BINDINGS) return this.#say(`每条命令最多 ${MAX_BINDINGS} 个绑定`, true);
    const owners = this.#model.commands.filter(
      (c) => refKey(c) !== refKey(info) && c.scope === info.scope && c.current.some((each) => sameKey(each, input)),
    );
    if (owners.length) {
      this.#pending = { input, owners, target: { plugin: info.plugin, command: info.command } };
      this.#say("");
      return;
    }
    await this.#apply(input, [], info);
    // The other scope keeps its own use (panel bindings are looked up first).
    const other = this.#model.commands.find(
      (c) => c.scope !== info.scope && c.current.some((each) => sameKey(each, input)),
    );
    if (other && !this.#message.warn)
      this.#say(
        info.scope === "global"
          ? `此键在「${sourceText(other.plugin)}」面板获得焦点时另有用途：${commandText(other)}`
          : `此键在面板外另有用途：${commandText(other)}`,
      );
  }

  #pendingText() {
    const pending = this.#pending;
    if (!pending) return "";
    const target = this.#model.text(pending.target);
    if (pending.owners.some((o) => o.plugin === "host" && o.command === "quit"))
      return `⌘Q 是退出 Vestra Player 的默认快捷键，确定改为「${target}」？`;
    const first =
      pending.owners.find((o) => !o.shadowed.some((s) => sameKey(s.input, pending.input))) ?? pending.owners[0]!;
    return `${label(pending.input)} 已用于「${commandText(first)}」`;
  }

  async #replace() {
    const pending = this.#pending;
    this.#pending = null;
    if (!pending) return;
    // The owners and the target as the list knows them now.
    const owners = pending.owners.map((o) => this.#model.find(o)).filter((o): o is BindingInfo => !!o);
    const target = this.#model.find(pending.target);
    if (!target) return this.#say("该命令已不存在", true);
    await this.#apply(pending.input, owners, target);
  }

  /** Take `input` from `owners`, add it to `target`, report. */
  async #apply(input: string, owners: BindingInfo[], target: BindingInfo) {
    try {
      await this.#model.move(input, owners, target, (each) => sameKey(each, input));
      this.#updated(target);
    } catch (error) {
      this.#say(failureText(error), true);
    }
  }

  #updated(ref: BindingRef) {
    const info = this.#model.find(ref);
    if (!info) return;
    const keys = info.labels.join(", ") || "（无）";
    this.#say(`已更新：${keys}`);
    this.#osd(`快捷键已更新：${commandText(info)} = ${keys}`);
  }

  async #write(action: () => Promise<void>, ref: BindingRef) {
    try {
      await action();
      await this.#model.reload();
      this.#updated(ref);
    } catch (error) {
      this.#say(failureText(error), true);
      await this.#model.reload().catch(() => {});
    }
  }

  #removeTag(input: string) {
    const info = this.#selectedLine()?.info;
    if (!info) return;
    this.#pending = null;
    return this.#write(() => this.#model.write(info, info.current.filter((each) => each !== input)), info);
  }

  async #resetOne() {
    const line = this.#selectedLine();
    if (!line) return;
    this.#pending = null;
    if (!line.info) {
      // 未加载的插件: drop its override for good.
      try {
        await this.#model.reset(line.ref);
        await this.#model.reload();
        this.#selected = null;
        this.#say(`已删除 ${refKey(line.ref)} 的修改`);
      } catch (error) {
        this.#say(failureText(error), true);
      }
      return;
    }
    return this.#write(() => this.#model.reset(line.ref), line.ref);
  }

  #unbindAll() {
    const info = this.#selectedLine()?.info;
    if (!info) return;
    this.#pending = null;
    return this.#write(() => this.#model.write(info, []), info);
  }

  async #resetAll() {
    let answer;
    try {
      answer = await this.#plugin.dialogs.confirm({
        message: "恢复全部默认快捷键？",
        detail: "将清除你对所有命令（包括未加载插件）的快捷键与鼠标动作修改。",
        confirm: "恢复全部默认",
        destructive: true,
      });
    } catch (error) {
      return this.#say(`无法确认：${(error as Error).message}`, true);
    }
    if (!answer.confirmed) return;
    this.#pending = null;
    try {
      await this.#model.reset();
      await this.#model.reload();
      this.#say("已恢复全部默认");
      this.#osd("快捷键已恢复默认");
    } catch (error) {
      this.#say(failureText(error), true);
    }
  }

  /** 导出列表…: the full table as CSV (`.csv`) or aligned text. */
  async #export() {
    let path: string | null;
    try {
      path = await this.#plugin.dialogs.saveFile({ title: "导出快捷键列表", fileName: "Vestra Player 快捷键.csv" });
    } catch (error) {
      return this.#say(`无法导出：${(error as Error).message}`, true);
    }
    if (!path) return;
    const lines = this.lines()
      .filter((line) => !!line.info)
      .sort((a, b) => a.command.localeCompare(b.command, "zh-Hans-CN"));
    const text = path.toLowerCase().endsWith(".csv") ? exportCsv(lines) : exportText(lines);
    try {
      const temp = `${path}.tmp-${process.pid}`;
      writeFileSync(temp, text);
      renameSync(temp, path);
      this.#say(`已导出 ${lines.length} 条命令`);
    } catch (error) {
      this.#say(`无法导出：${(error as Error).message}`, true);
    }
  }
}
