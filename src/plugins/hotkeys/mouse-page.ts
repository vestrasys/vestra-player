import type { BindingInfo, Plugin } from "@vestra-player/sdk";
import { inputOf, type Message, type Modifier, MODIFIERS, MOUSE, message, refKey, winner } from "./keys.ts";
import type { Osd } from "./main.ts";
import { type BindingsModel, failure } from "./model.ts";

/** The J02 page in the options window (category Keys and Mouse). */
export const MOUSE_PAGE = "hotkeys.mouse";

/** `Ctrl+Mouse:WheelUp` for a modifier choice and a mouse input. */
export const mouseInput = (modifier: Modifier, input: string) => (modifier ? `${modifier}+${input}` : input);

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
  /** The system has ⌘ (macOS): `Cmd` is always offered (`hasCmd`). */
  readonly #mac: boolean;
  #modifier: Modifier = "";
  /** Row being picked for, or null. */
  #picking: number | null = null;
  #filter = "";
  #message = message();

  constructor(plugin: Plugin, model: BindingsModel, osd: Osd, mac: boolean) {
    this.#mac = mac;
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
      const modifier = this.#modifiers()[index];
      if (modifier === undefined) return;
      this.#modifier = modifier;
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

  /** Mouse… (command palette, Settings: …): the options window on this page, with a fresh list. */
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

  /**
   * The modifier radios. `Cmd` only on macOS: elsewhere it is the Windows
   * (Super) key, which the menus never show either. A `Cmd` mouse binding
   * kept from elsewhere (a synced or older setting) still brings the radio
   * back, so it can be seen and changed; it stays while chosen.
   */
  #modifiers(): Modifier[] {
    const cmd =
      this.#mac ||
      this.#modifier === "Cmd" ||
      this.#global().some((c) => c.current.some((input) => MOUSE.some((m) => input === mouseInput("Cmd", m.input))));
    return MODIFIERS.filter((m) => m !== "Cmd" || cmd);
  }

  rows() {
    return MOUSE.map((mouse, index) => {
      const input = mouseInput(this.#modifier, mouse.input);
      const owner = winner(this.#model.commands, input, "global");
      return {
        input: index,
        command: owner ? this.#model.commandText(owner) : "",
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
    const modifiers = this.#modifiers();
    const items =
      row === null
        ? []
        : [
            { key: "", text: "", selected: !owner },
            ...this.#global()
              .map((c) => ({ key: refKey(c), text: this.#model.commandText(c), selected: !!owner && refKey(owner) === refKey(c) }))
              .sort((a, b) => a.text.localeCompare(b.text, "zh-Hans-CN")),
          ].filter((item) => !filter || item.key === "" || item.text.toLowerCase().includes(filter));
    return {
      // The radios show their labels: None translated, the modifiers as keys name them.
      modifiers: modifiers.map((m) => (m ? m : this.#plugin.i18n.text("None"))),
      modifier: modifiers.indexOf(this.#modifier),
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
    const command = owner ? this.#model.commandText(owner) : "";
    this.#say(message(owner ? "mouse-updated" : "mouse-updated-none", { input: inputOf(input), text: command }));
    const { text } = this.#plugin.i18n;
    const modifier = this.#modifier;
    const mouse = text(MOUSE[row]!.label);
    const name = modifier ? text("{}+{}", [modifier, mouse]) : mouse;
    this.#osd(
      owner
        ? this.#plugin.osd.show("Mouse action updated: {0} = {1}", [name, command])
        : this.#plugin.osd.show("Mouse action updated: {} = (none)", [name]),
    );
  }
}
