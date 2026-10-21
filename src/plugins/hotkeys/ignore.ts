import type { BindingRef, JsonStore, Plugin } from "@vestra-player/sdk";
import { exempt, refKey, sameRef } from "./keys.ts";
import type { BindingsModel } from "./model.ts";
import { type IgnoreRecord, jsonStore, readRecord } from "./store.ts";

/**
 * J03 忽略默认快捷键, as ruled in R-K2: no player switch. While on, every
 * command that still runs on its defaults gets an empty override, and
 * ignore-defaults.json records which overrides this plugin wrote. Turning
 * it off resets only those (a command the user changed meanwhile left the
 * record). Commands that appear later (a plugin loaded or enabled) are
 * caught on `bindings.onChange`. 退出 and 退出全屏 are never ignored.
 */
export class IgnoreDefaults {
  readonly #plugin: Plugin;
  readonly #model: BindingsModel;
  readonly #osd: (text: string) => void;
  readonly #store: JsonStore<IgnoreRecord>;
  #record: IgnoreRecord;
  #queue: Promise<void> = Promise.resolve();
  #again = false;
  #pending = false;

  constructor(plugin: Plugin, model: BindingsModel, osd: (text: string) => void) {
    this.#plugin = plugin;
    this.#model = model;
    this.#osd = osd;
    this.#store = jsonStore<IgnoreRecord>(plugin, "ignore-defaults", { version: 1, enabled: false, written: [] });
    this.#record = readRecord(this.#store.value);
    model.onWrite = (ref) => this.forget(ref);
    plugin.bindings.onChange(() => this.sync());
  }

  get enabled() {
    return this.#record.enabled;
  }
  /** The command's defaults are switched off by J03 (shown as 已忽略). */
  ignores(ref: BindingRef) {
    return this.#record.written.some((each) => sameRef(each, ref));
  }

  /** The menu item and the J01 check box: ask before turning it on. */
  async toggle() {
    if (this.enabled) return this.set(false);
    const answer = await this.#plugin.dialogs.confirm({
      message: "忽略默认快捷键？",
      detail:
        "忽略后，只有你在快捷键设置中修改过的按键生效，Space、方向键等默认按键都将失效。「退出」（⌘Q）与「退出全屏」（Esc）不受影响，可随时从「设置 ▶ 忽略默认快捷键」关闭。",
      confirm: "忽略",
    });
    if (answer.confirmed) await this.set(true);
  }

  async set(enabled: boolean) {
    this.#save({ ...this.#record, enabled });
    await this.sync();
    this.#osd(`忽略默认快捷键：${enabled ? "开" : "关"}`);
  }

  /** The user changed this command: its override is theirs now. */
  forget(ref: BindingRef) {
    if (!this.ignores(ref)) return;
    this.#save({ ...this.#record, written: this.#record.written.filter((each) => !sameRef(each, ref)) });
  }

  /**
   * Bring the overlay in line with the switch (at start, after each change).
   * Runs one at a time; changes during a run cause one more run.
   */
  sync(): Promise<void> {
    if (!this.#record.enabled && this.#record.written.length === 0) return Promise.resolve();
    if (this.#pending) {
      this.#again = true;
      return this.#queue;
    }
    this.#pending = true;
    this.#queue = this.#queue.then(async () => {
      do {
        this.#again = false;
        await this.#reconcile().catch(() => {});
      } while (this.#again);
      this.#pending = false;
    });
    return this.#queue;
  }

  async #reconcile() {
    const { commands, orphans } = await this.#plugin.bindings.list();
    const bindings = this.#plugin.bindings;
    if (!this.#record.enabled) {
      // Off: undo what is still ours (an empty override nobody touched).
      for (const ref of this.#record.written) {
        const info = commands.find((c) => sameRef(c, ref));
        const orphan = orphans.find((o) => sameRef(o.ref, ref));
        const untouched = info ? info.overridden && info.current.length === 0 : orphan?.inputs.length === 0;
        if (untouched) await bindings.reset({ plugin: ref.plugin, command: ref.command }).catch(() => {});
      }
      this.#save({ ...this.#record, written: [] });
      return;
    }
    let changed = false;
    for (const c of commands) {
      if (exempt(c)) continue;
      if (!c.overridden && c.defaults.length) {
        try {
          await bindings.set({ plugin: c.plugin, command: c.command }, []);
          if (!this.ignores(c)) this.#save({ ...this.#record, written: [...this.#record.written, { plugin: c.plugin, command: c.command }] });
          changed = true;
        } catch {
          // Gone meanwhile; the next change notice brings the next run.
        }
      } else if (this.ignores(c) && c.overridden && c.current.length) {
        this.forget(c); // rebound elsewhere: the user's now
      }
    }
    // Neither declared nor kept as an orphan: the override was cleared.
    const known = new Set([...commands.map(refKey), ...orphans.map((o) => refKey(o.ref))]);
    const live = this.#record.written.filter((ref) => known.has(refKey(ref)));
    if (live.length !== this.#record.written.length) this.#save({ ...this.#record, written: live });
    // Our own writes come back as change notices; they find nothing to do.
    if (changed) this.#model.schedule();
  }

  #save(next: IgnoreRecord) {
    this.#record = next;
    this.#store.save(next);
    this.#plugin.update();
  }
}
