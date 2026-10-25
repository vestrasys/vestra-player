import type { BindingInfo, Plugin } from "@vestra-player/sdk";
import { commandText, inputOf, type Message, MODIFIERS, MOUSE, message, refKey, winner } from "./keys.ts";
import type { Osd } from "./main.ts";
import { type BindingsModel, failure } from "./model.ts";

/** The J02 page in the options window (category Keys and Mouse). */
export const MOUSE_PAGE = "hotkeys.mouse";

/** `Ctrl+Mouse:WheelUp` for a modifier choice and a mouse input. */
export const mouseInput = (modifier: number, input: string) =>
  MODIFIERS[modifier] ? `${MODIFIERS[modifier]}+${input}` : input;

/**
 * Mouse (J02), a page of the options window: for each mouse input (under the chosen modifier) the
 * global command it runs. Choosing a command first takes the input from
 * whoever holds it, then adds it to the chosen one ((None) only takes it).
 * Default gives the input back to the command that declares it. Texts are
 * in ui/mouse.slint; this passes data.
 */
export class MousePage {
  readonly #plugin: Plugin;
  readonly #model: BindingsModel;
  readonly #osd: Osd;
  readonly #page;
  #modifier = 0;
  /** Row being picked for, or null. */
  #picking: number | null = null;
  #filter = "";
  #message = message();

  constructor(plugin: Plugin, model: BindingsModel, osd: Osd) {
    this.#plugin = plugin;
    this.#model = model;
    this.#osd = osd;
    const page = (this.#page = plugin.settings.page("ui/mouse.slint", {
      id: MOUSE_PAGE,
      category: "input",
      title: "Mouse",
      order: 20,
    }));
    page.on("setModifier", (index: number) => {
      if (index < 0 || index >= MODIFIERS.length) return;
      this.#modifier = index;
      this.#picking = null;
    });
    page.on("pick", (row: number) => {
      if (!MOUSE[row]) return;
      this.#picking = row;
      this.#filter = "";
      page.set({ pickFilter: "" });
    });
    page.on("filter", (text: string) => {
      this.#filter = text;
    });
    page.on("cancelPick", () => {
      this.#picking = null;
    });
    page.on("choose", (key: string) => this.#choose(key));
    page.on("setDefault", (row: number) => this.#default(row));
    page.bind(() => this.#props());
  }

  /** Settings ▸ Mouse…: the options window on this page, with a fresh list. */
  async show() {
    await this.#model.reload().catch((error) => this.#say(failure(error)));
    await this.#plugin.settings.show(MOUSE_PAGE);
  }

  #say(next: Message) {
    this.#message = next;
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
    return MOUSE.map((mouse, index) => {
      const input = mouseInput(this.#modifier, mouse.input);
      const owner = winner(this.#model.commands, input, "global");
      return {
        input: index,
        command: owner ? commandText(owner) : "",
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
            { key: "", text: "", selected: !owner },
            ...this.#global()
              .map((c) => ({ key: refKey(c), text: commandText(c), selected: !!owner && refKey(owner) === refKey(c) }))
              .sort((a, b) => a.text.localeCompare(b.text, "zh-Hans-CN")),
          ].filter((item) => !filter || item.key === "" || item.text.toLowerCase().includes(filter));
    return {
      modifiers: [...MODIFIERS],
      modifier: this.#modifier,
      rows: this.rows(),
      picking: row !== null,
      pickInput: inputOf(input ?? ""),
      pickItems: items,
      message: this.#message,
    };
  }

  async #choose(key: string) {
    const row = this.#picking;
    this.#picking = null;
    if (row === null) return;
    const input = mouseInput(this.#modifier, MOUSE[row]!.input);
    const target = key ? (this.#global().find((c) => refKey(c) === key) ?? null) : null;
    if (key && !target) return this.#say(message("missing", { warn: true }));
    await this.#move(row, input, target);
  }

  /** Default: the declaring command gets the input back; anyone else loses it. */
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
      this.#say(failure(error));
      await this.#model.reload().catch(() => {});
    }
  }

  async #move(row: number, input: string, target: BindingInfo | null) {
    try {
      await this.#model.move(input, this.#claimers(input), target, (each) => each === input);
      this.#updated(row, input);
    } catch (error) {
      this.#say(failure(error));
    }
  }

  #updated(row: number, input: string) {
    const owner = winner(this.#model.commands, input, "global");
    const command = owner ? commandText(owner) : "";
    this.#say(message(owner ? "mouse-updated" : "mouse-updated-none", { input: inputOf(input), text: command }));
    const { text } = this.#plugin.i18n;
    const modifier = MODIFIERS[this.#modifier];
    const mouse = text(MOUSE[row]!.label);
    const name = modifier ? text("{}+{}", [modifier, mouse]) : mouse;
    this.#osd(
      owner
        ? this.#plugin.osd.show("Mouse action updated: {0} = {1}", [name, command])
        : this.#plugin.osd.show("Mouse action updated: {} = (none)", [name]),
    );
  }
}
