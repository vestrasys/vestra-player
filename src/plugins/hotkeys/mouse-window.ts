import type { BindingInfo, Plugin } from "../../sdk/plugin/index.ts";
import { commandText, MODIFIERS, MOUSE, refKey, winner } from "./keys.ts";
import { type BindingsModel, failureText } from "./model.ts";

const NONE = "（无）";

/** `Ctrl+Mouse:WheelUp` for a modifier choice and a mouse input. */
export const mouseInput = (modifier: number, input: string) =>
  MODIFIERS[modifier] ? `${MODIFIERS[modifier]}+${input}` : input;

/**
 * 鼠标动作 (J02): for each mouse input (under the chosen modifier) the
 * global command it runs. Choosing a command first takes the input from
 * whoever holds it, then adds it to the chosen one (（无） only takes it).
 * 默认 gives the input back to the command that declares it.
 */
export class MouseWindow {
  readonly #model: BindingsModel;
  readonly #osd: (text: string) => void;
  readonly #window;
  #modifier = 0;
  /** Row being picked for, or null. */
  #picking: number | null = null;
  #filter = "";
  #message = { text: "", warn: false };

  constructor(plugin: Plugin, model: BindingsModel, osd: (text: string) => void) {
    this.#model = model;
    this.#osd = osd;
    const window = (this.#window = plugin.windows.create("ui/mouse.slint"));
    window.on("setModifier", (index: number) => {
      if (index < 0 || index >= MODIFIERS.length) return;
      this.#modifier = index;
      this.#picking = null;
    });
    window.on("pick", (row: number) => {
      if (!MOUSE[row]) return;
      this.#picking = row;
      this.#filter = "";
      window.set({ pickFilter: "" });
    });
    window.on("filter", (text: string) => {
      this.#filter = text;
    });
    window.on("cancelPick", () => {
      this.#picking = null;
    });
    window.on("choose", (key: string) => this.#choose(key));
    window.on("setDefault", (row: number) => this.#default(row));
    window.on("close", () => window.close());
    window.onClose(() => {
      this.#picking = null;
    });
    window.bind(() => this.#props());
  }

  get opened() {
    return this.#window.opened;
  }

  /** 设置 ▶ 鼠标…: open with a fresh list. */
  async show() {
    await this.#model.reload().catch((error) => this.#say(failureText(error), true));
    await this.#window.show();
  }

  #say(text: string, warn = false) {
    this.#message = { text, warn };
  }

  #global(): BindingInfo[] {
    return this.#model.commands.filter((c) => c.scope === "global");
  }
  /** Commands holding `input` (normally one). */
  #claimers(input: string) {
    return this.#global().filter((c) => c.current.includes(input));
  }
  /** The command that declares `input` (its default home). */
  #home(input: string) {
    return this.#global().find((c) => c.defaults.includes(input)) ?? null;
  }
  /** The input sits where its default puts it (or nowhere, without a default). */
  #isDefault(input: string) {
    const claimers = this.#claimers(input);
    const home = this.#home(input);
    if (!home) return claimers.length === 0;
    return claimers.length === 1 && refKey(claimers[0]!) === refKey(home);
  }

  rows() {
    return MOUSE.map((mouse) => {
      const input = mouseInput(this.#modifier, mouse.input);
      const owner = winner(this.#model.commands, input, "global");
      return {
        label: mouse.label,
        command: owner ? commandText(owner) : NONE,
        bound: !!owner,
        resettable: !this.#isDefault(input),
      };
    });
  }

  #props() {
    const row = this.#picking;
    const input = row === null ? null : mouseInput(this.#modifier, MOUSE[row]!.input);
    const owner = input ? winner(this.#model.commands, input, "global") : null;
    const filter = this.#filter.trim().toLowerCase();
    const items =
      row === null
        ? []
        : [
            { key: "", text: NONE, selected: !owner },
            ...this.#global()
              .map((c) => ({ key: refKey(c), text: commandText(c), selected: !!owner && refKey(owner) === refKey(c) }))
              .sort((a, b) => a.text.localeCompare(b.text, "zh-Hans-CN")),
          ].filter((item) => !filter || item.key === "" || item.text.toLowerCase().includes(filter));
    return {
      modifiers: MODIFIERS.map((m) => m || "无"),
      modifier: this.#modifier,
      rows: this.rows(),
      picking: row !== null,
      pickTitle: row === null ? "" : `「${this.#inputName(row)}」触发的命令：`,
      pickItems: items,
      message: this.#message.text,
      messageWarn: this.#message.warn,
    };
  }

  #inputName(row: number) {
    const modifier = MODIFIERS[this.#modifier];
    return `${modifier ? `${modifier}+` : ""}${MOUSE[row]!.label}`;
  }

  async #choose(key: string) {
    const row = this.#picking;
    this.#picking = null;
    if (row === null) return;
    const input = mouseInput(this.#modifier, MOUSE[row]!.input);
    const target = key ? (this.#global().find((c) => refKey(c) === key) ?? null) : null;
    if (key && !target) return this.#say("该命令已不存在", true);
    await this.#move(row, input, target);
  }

  /** 默认: the declaring command gets the input back; anyone else loses it. */
  async #default(row: number) {
    const mouse = MOUSE[row];
    if (!mouse) return;
    const input = mouseInput(this.#modifier, mouse.input);
    const home = this.#home(input);
    try {
      const others = this.#claimers(input).filter((c) => !home || refKey(c) !== refKey(home));
      await this.#model.move(input, others, null, (each) => each === input);
      const fresh = home && this.#model.find(home);
      if (fresh && !fresh.current.includes(input)) {
        const next = [...fresh.current, input];
        const same = next.length === fresh.defaults.length && next.every((each) => fresh.defaults.includes(each));
        await (same ? this.#model.reset(fresh) : this.#model.write(fresh, next));
        await this.#model.reload();
      }
      this.#updated(row, input);
    } catch (error) {
      this.#say(failureText(error), true);
      await this.#model.reload().catch(() => {});
    }
  }

  async #move(row: number, input: string, target: BindingInfo | null) {
    try {
      await this.#model.move(input, this.#claimers(input), target, (each) => each === input);
      this.#updated(row, input);
    } catch (error) {
      this.#say(failureText(error), true);
    }
  }

  #updated(row: number, input: string) {
    const owner = winner(this.#model.commands, input, "global");
    const text = `${this.#inputName(row)} = ${owner ? commandText(owner) : NONE}`;
    this.#say(`已更新：${text}`);
    this.#osd(`鼠标动作已更新：${text}`);
  }
}
