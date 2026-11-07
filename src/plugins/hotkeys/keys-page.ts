import { renameSync, writeFileSync } from "node:fs";
import type { BindingInfo, BindingRef, JsonStore, Plugin, TextArg } from "@vestra-player/sdk";
import { exportCsv, exportText } from "./export.ts";
import type { IgnoreDefaults } from "./ignore.ts";
import type { Osd } from "./main.ts";
import {
  type Conflict,
  fromSlintKey,
  inputOf,
  isMouse,
  label,
  type Message,
  message,
  refKey,
  STATE,
  sameKey,
} from "./keys.ts";
import { type BindingsModel, failure } from "./model.ts";
import { jsonStore, PREFS, type Prefs, readPrefs } from "./store.ts";

/** Status filter choices (J01; ui/keys.slint names them): all, changed, conflicts, unbound. */
const ORPHANS = "*orphans";
/** J01: at most this many bindings per command. */
export const MAX_BINDINGS = 8;

/** One row of the page's list (Command | Shortcut | Status). */
type Row = {
  key: string;
  command: string;
  /** Key labels (the player's), `, `-separated; an ignored command's defaults (shown in brackets). */
  keys: string;
  state: number;
  selected: boolean;
  conflicts: Conflict[];
};
/** One line of the table, for rows, the edit area and the exported list. */
export type Line = Omit<Row, "selected"> & {
  /** `""` for orphans. */
  scope: BindingInfo["scope"] | "";
  /** The plugin id, `host` for the player's own commands. */
  source: string;
  info: BindingInfo | null;
  ref: BindingRef;
  inputs: string[];
  /** Sort text of the keys column (an ignored command's defaults in brackets, as shown). */
  sortKeys: string;
  /**
   * What the search box matches besides the keys, lower case: the name as shown plus the
   * menu path and the category, so "Video" still finds Video ▸ Snapshot ▸ … now that the
   * name is "Snapshot: Save Current Frame".
   */
  search: string;
};

/** The J01 page in the options window (category Keys and Mouse). */
export const KEYS_PAGE = "hotkeys.keyboard";

/** Changed / conflict / ignored / default for one command (J01, J03). */
export function lineOf(info: BindingInfo, ignored: boolean, model: BindingsModel): Line {
  const keyboard = info.labels.filter((_, i) => !isMouse(info.current[i] ?? ""));
  let keys = keyboard.join(", ");
  let sortKeys = keys;
  let state: number = STATE.default;
  let conflicts: Conflict[] = [];
  if (ignored) {
    state = STATE.ignored;
    keys = info.defaults.filter((input) => !isMouse(input)).map(label).join(", ");
    sortKeys = keys && `（${keys}）`;
  } else if (info.shadowed.length) {
    state = STATE.conflict;
    conflicts = info.shadowed.map((s) => ({ input: inputOf(s.input, model.labelOf(s.input)), by: model.text(s.by) }));
  } else if (info.overridden) {
    state = STATE.changed;
  }
  const command = model.commandText(info);
  return {
    key: refKey(info),
    ref: { plugin: info.plugin, command: info.command },
    info,
    inputs: info.current,
    command,
    search: [command, ...(info.path ?? []), info.category ?? ""].join("\n").toLowerCase(),
    keys,
    sortKeys,
    scope: info.scope,
    source: info.plugin,
    state,
    conflicts,
  };
}

/**
 * Keyboard Shortcuts (J01), a page of the options window: every command of the binding table with its effective
 * keys, filtered and sorted; the selected command's bindings as tags that
 * can be removed, a capture box to add one (conflicts ask Replace / Cancel),
 * Restore Default / Unbind All, and Restore All Defaults… / Export List… /
 * Ignore default shortcuts. Texts are in ui/keys.slint; this passes data.
 * The list follows `bindings.onChange`; the selection stays by command.
 */
export class KeysPage {
  readonly #plugin: Plugin;
  readonly #model: BindingsModel;
  readonly #ignore: IgnoreDefaults;
  readonly #osd: Osd;
  readonly #page;
  readonly #prefs: JsonStore<Prefs>;
  #search = "";
  #selected: string | null = null;
  #capturing = false;
  #pending: { input: string; owners: BindingInfo[]; target: BindingRef } | null = null;
  #message = message();
  #rows: { key: string; rows: Row[]; lines: Line[] } = { key: "", rows: [], lines: [] };
  /** The system has ⌘ (macOS); elsewhere Slint's `meta` is the Windows key (`fromSlintKey`). */
  readonly #mac: boolean;

  constructor(plugin: Plugin, model: BindingsModel, ignore: IgnoreDefaults, osd: Osd, mac: boolean) {
    this.#mac = mac;
    this.#plugin = plugin;
    this.#model = model;
    this.#ignore = ignore;
    this.#osd = osd;
    this.#prefs = jsonStore<Prefs>(plugin, "hotkeys", PREFS);
    const page = (this.#page = plugin.settings.page("ui/keys.slint", {
      id: KEYS_PAGE,
      category: "input",
      title: "Keyboard Shortcuts",
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
      this.#say();
    });
    page.on("resetAll", () => this.#resetAll());
    page.on("export", () => this.#export());
    // The check box does not flip itself (`self-toggle: false`); the plugin decides
    // (turning it on asks first) and pushes the state back afterwards, also when it
    // did not change (a declined question) or the toggle failed.
    page.on("toggleIgnore", async () => {
      try {
        await ignore.toggle();
      } finally {
        page.set({ ignore: ignore.enabled });
      }
    });
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

  /** Settings ▸ Keyboard Shortcuts…: the options window on this page, with a fresh list. */
  async show() {
    await this.#model.reload().catch((error) => this.#say(failure(error)));
    await this.#plugin.settings.show(KEYS_PAGE);
  }

  #prefsValue(): Prefs {
    return readPrefs(this.#prefs.value);
  }
  #savePrefs(patch: Partial<Prefs>) {
    this.#prefs.save({ ...this.#prefsValue(), ...patch });
  }

  /**
   * Source filter: all, the player, each plugin with commands, plugins not
   * loaded. `title` is what the Slint drop-down shows: plugin ids as they
   * are, the other entries translated (the library Choice shows its model).
   */
  #sources(): { title: TextArg; value: string }[] {
    const i18n = this.#plugin.i18n;
    const plugins = [...new Set(this.#model.commands.map((c) => c.plugin))]
      .filter((id) => id !== "host")
      .sort();
    return [
      { title: i18n.text("All sources"), value: "" },
      { title: i18n.text("Player"), value: "host" },
      ...plugins.map((id) => ({ title: id, value: id })),
      { title: i18n.text("Plugins not loaded"), value: ORPHANS },
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
        search: refKey(o.ref).toLowerCase(),
        keys: o.inputs.filter((i) => !isMouse(i)).map(label).join(", "),
        sortKeys: o.inputs.filter((i) => !isMouse(i)).map(label).join(", "),
        scope: "",
        source: o.ref.plugin,
        state: STATE.orphan,
        conflicts: [],
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
        if (prefs.status === 1) return line.state === STATE.changed || line.state === STATE.ignored;
        if (prefs.status === 2) return line.state === STATE.conflict;
        if (prefs.status === 3) return line.inputs.length === 0;
        return true;
      })
      .filter(
        (line) =>
          !search ||
          line.search.includes(search) ||
          line.keys.toLowerCase().includes(search) ||
          line.inputs.some((input) => input.toLowerCase().includes(search)),
      );
    const by = prefs.sort === "keys" ? (line: Line) => line.sortKeys : (line: Line) => line.command;
    lines.sort((a, b) => {
      // Unbound commands go last when sorting by keys.
      if (prefs.sort === "keys" && !a.keys !== !b.keys) return a.keys ? -1 : 1;
      return by(a).localeCompare(by(b), "zh-Hans-CN") || a.command.localeCompare(b.command, "zh-Hans-CN");
    });
    if (prefs.descending) lines.reverse();
    const { text, resolve } = this.#plugin.i18n;
    const rows = lines.map(({ key, command, scope, keys, state, conflicts }) => ({
      key,
      // A panel's own command often shares its name with the menu item it mirrors
      // ("Playlist: Add Files…" twice): the row says which one works only in the panel,
      // as the edit area below does.
      command: scope === "panel" ? resolve(text("{} (in panel)", [command])) : command,
      keys,
      state,
      conflicts,
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
      loading: this.#model.commands.length === 0,
      statusIndex: prefs.status,
      sourceModel: sources.map((s) => s.title),
      sourceIndex: Math.max(
        0,
        sources.findIndex((s) => s.value === prefs.source),
      ),
      sort: prefs.sort,
      descending: prefs.descending,
      hasSelection: !!line,
      editCommand: line?.command ?? "",
      editSource: line?.source ?? "",
      editPanel: info?.scope === "panel",
      editLoaded: !!info,
      tags: info ? info.current.map((input, i) => ({ input, label: inputOf(input, info.labels[i]) })) : [],
      canAdd: !!info,
      canReset: !!line && (!info || info.overridden),
      orphan: !!line && !info,
      canUnbind: !!info && info.current.length > 0,
      capturing: this.#capturing,
      pending: !!this.#pending,
      ...this.#pendingText(),
      message: this.#message,
      ignore: this.#ignore.enabled,
    };
  }

  #say(next: Message = message()) {
    this.#message = next;
  }

  #select(key: string) {
    if (key === this.#selected) return;
    this.#selected = key;
    this.#capturing = false;
    this.#pending = null;
    const line = this.#selectedLine();
    this.#say(line?.state === STATE.conflict ? message("conflict", { conflicts: line.conflicts }) : undefined);
  }

  #startCapture() {
    if (!this.#selectedLine()?.info) return;
    this.#pending = null;
    this.#capturing = true;
    this.#say(message("capture"));
  }

  #key(event: Parameters<typeof fromSlintKey>[0]) {
    if (!this.#capturing) return;
    const result = fromSlintKey(event, this.#mac);
    if (!result) return; // a modifier alone (or the Windows key held): keep waiting
    if ("cancel" in result) {
      this.#capturing = false;
      this.#say();
      return;
    }
    if ("error" in result) {
      this.#capturing = false;
      this.#say(message(result.error, { warn: true }));
      return;
    }
    return this.#captured(result.input);
  }

  /** A key was captured for the selected command. */
  async #captured(input: string) {
    this.#capturing = false;
    const info = this.#selectedLine()?.info;
    if (!info) return;
    if (info.current.includes(input)) return this.#say(message("already", { input: inputOf(input, this.#model.labelOf(input)) }));
    if (info.current.length >= MAX_BINDINGS) return this.#say(message("max", { n: MAX_BINDINGS, warn: true }));
    const owners = this.#model.commands.filter(
      (c) => refKey(c) !== refKey(info) && c.scope === info.scope && c.current.some((each) => sameKey(each, input)),
    );
    if (owners.length) {
      this.#pending = { input, owners, target: { plugin: info.plugin, command: info.command } };
      this.#say();
      return;
    }
    await this.#apply(input, [], info);
    // The other scope keeps its own use (panel bindings are looked up first).
    const other = this.#model.commands.find(
      (c) => c.scope !== info.scope && c.current.some((each) => sameKey(each, input)),
    );
    if (other && !this.#message.warn) {
      const text = this.#model.commandText(other);
      if (info.scope !== "global") this.#say(message("outside-use", { text }));
      else if (other.plugin === "host") this.#say(message("player-panel-use", { text }));
      else this.#say(message("panel-use", { text, other: other.plugin }));
    }
  }

  /** The question while a captured key is in use (ui/keys.slint words it). */
  #pendingText() {
    const pending = this.#pending;
    if (!pending) return { pendingKind: "", pendingKey: "", pendingCommand: "" };
    if (pending.owners.some((o) => o.plugin === "host" && o.command === "quit"))
      return { pendingKind: "quit", pendingKey: "", pendingCommand: this.#model.text(pending.target) };
    const first =
      pending.owners.find((o) => !o.shadowed.some((s) => sameKey(s.input, pending.input))) ?? pending.owners[0]!;
    return { pendingKind: "used", pendingKey: this.#model.labelOf(pending.input), pendingCommand: this.#model.commandText(first) };
  }

  async #replace() {
    const pending = this.#pending;
    this.#pending = null;
    if (!pending) return;
    // The owners and the target as the list knows them now.
    const owners = pending.owners.map((o) => this.#model.find(o)).filter((o): o is BindingInfo => !!o);
    const target = this.#model.find(pending.target);
    if (!target) return this.#say(message("missing", { warn: true }));
    await this.#apply(pending.input, owners, target);
  }

  /** Take `input` from `owners`, add it to `target`, report. */
  async #apply(input: string, owners: BindingInfo[], target: BindingInfo) {
    try {
      await this.#model.move(input, owners, target, (each) => sameKey(each, input));
      this.#updated(target);
    } catch (error) {
      this.#say(failure(error));
    }
  }

  #updated(ref: BindingRef) {
    const info = this.#model.find(ref);
    if (!info) return;
    // The keys, as the row shows them: a mouse input (Play/Pause's left
    // click, player 0.5.2) is the mouse page's.
    const keys = info.labels.filter((_, i) => !isMouse(info.current[i] ?? "")).join(", ");
    if (keys) {
      this.#say(message("updated", { text: keys }));
      this.#osd(this.#plugin.osd.show("Shortcut updated: {0} = {1}", [this.#model.commandText(info), keys]));
    } else {
      this.#say(message("updated-none"));
      this.#osd(this.#plugin.osd.show("Shortcut updated: {} = (none)", [this.#model.commandText(info)]));
    }
  }

  async #write(action: () => Promise<void>, ref: BindingRef) {
    try {
      await action();
      await this.#model.reload();
      this.#updated(ref);
    } catch (error) {
      this.#say(failure(error));
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
      // A plugin not loaded: drop its override for good.
      try {
        await this.#model.reset(line.ref);
        await this.#model.reload();
        this.#selected = null;
        this.#say(message("deleted", { text: refKey(line.ref) }));
      } catch (error) {
        this.#say(failure(error));
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
        message: "Restore all default shortcuts?",
        detail: "This clears your changes to the shortcuts and mouse actions of every command, including those of plugins that are not loaded.",
        confirm: "Restore All Defaults",
        destructive: true,
      });
    } catch (error) {
      return this.#say(message("confirm-failed", { text: (error as Error).message, warn: true }));
    }
    if (!answer.confirmed) return;
    this.#pending = null;
    try {
      await this.#model.reset();
      await this.#model.reload();
      this.#say(message("restored"));
      this.#osd(this.#plugin.osd.show("Shortcuts restored to defaults"));
    } catch (error) {
      this.#say(failure(error));
    }
  }

  /** Export List…: the full table as CSV (`.csv`) or aligned text. */
  async #export() {
    let path: string | null;
    try {
      // TRANSLATORS: only the part before the extension.
      path = await this.#plugin.dialogs.saveFile({ title: "Export Shortcut List", fileName: this.#plugin.i18n.text("Vestra Player Shortcuts.csv") });
    } catch (error) {
      return this.#say(message("export-failed", { text: (error as Error).message, warn: true }));
    }
    if (!path) return;
    const lines = this.lines()
      .filter((line) => !!line.info)
      .sort((a, b) => a.command.localeCompare(b.command, "zh-Hans-CN"));
    const text = path.toLowerCase().endsWith(".csv") ? exportCsv(this.#plugin.i18n, lines) : exportText(this.#plugin.i18n, lines);
    try {
      const temp = `${path}.tmp-${process.pid}`;
      writeFileSync(temp, text);
      renameSync(temp, path);
      this.#say(message("exported", { n: lines.length }));
    } catch (error) {
      this.#say(message("export-failed", { text: (error as Error).message, warn: true }));
    }
  }
}
