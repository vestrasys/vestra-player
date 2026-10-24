import type { BindingInfo, BindingRef, Plugin } from "@vestra-player/sdk";
import { commandText, isMouse, label, refKey } from "./keys.ts";
import { timing } from "./store.ts";

const sameSet = (a: string[], b: string[]) => a.length === b.length && a.every((each) => b.includes(each));

type Orphan = { ref: BindingRef; inputs: string[] };

/** User-facing text for a refused write. */
export function failureText(error: unknown): string {
  const code = (error as { code?: string }).code;
  if (code === "missing") return "该命令已不存在";
  if (code === "invalid") return "无法使用此按键";
  return `修改失败：${(error as Error).message}`;
}

/**
 * The player's binding table as last listed (`bindings.list`), refreshed
 * after `bindings.onChange` (merged over `timing.reload`). Both windows and
 * J03 read it; writes go through `write`, which tells J03 the user changed
 * a command (its ignore record must not undo that).
 */
export class BindingsModel {
  readonly #plugin: Plugin;
  commands: BindingInfo[] = [];
  orphans: Orphan[] = [];
  /** Bumped by every reload; windows rebuild rows when it moves. */
  version = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #loading: Promise<void> | null = null;
  readonly #listeners = new Set<() => void>();
  /** Called before each user write (J03 forgets the command). */
  onWrite: (ref: BindingRef) => void = () => {};

  constructor(plugin: Plugin) {
    this.#plugin = plugin;
    plugin.bindings.onChange(() => this.schedule());
    plugin.onDispose(() => {
      if (this.#timer) clearTimeout(this.#timer);
    });
  }

  onChange(listener: () => void) {
    this.#listeners.add(listener);
  }

  schedule() {
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.reload().catch(() => {});
    }, timing.reload);
  }

  /** List again; concurrent calls share one request. */
  reload(): Promise<void> {
    this.#loading ??= (async () => {
      try {
        const table = await this.#plugin.bindings.list();
        this.commands = table.commands;
        this.orphans = table.orphans;
        this.version++;
      } finally {
        this.#loading = null;
      }
      for (const listener of this.#listeners) listener();
      this.#plugin.update();
    })();
    return this.#loading;
  }

  find(ref: BindingRef): BindingInfo | undefined {
    return this.commands.find((c) => c.plugin === ref.plugin && c.command === ref.command);
  }
  /**
   * Display text of a keyboard input: the player's own label when some
   * command is bound to it now (the system's menu style, `⌘O` on macOS),
   * otherwise `label` (ignored defaults, orphans, keys bound to nothing).
   */
  labelOf(input: string): string {
    if (!isMouse(input))
      for (const command of this.commands) {
        const shown = command.labels[command.current.indexOf(input)];
        if (shown) return shown;
      }
    return label(input);
  }
  orphan(ref: BindingRef): Orphan | undefined {
    return this.orphans.find((o) => refKey(o.ref) === refKey(ref));
  }
  text(ref: BindingRef): string {
    const info = this.find(ref);
    return info ? commandText(info) : refKey(ref);
  }

  /** A user's change: `inputs` for one command (`[]` unbinds it). */
  async write(ref: BindingRef, inputs: string[]) {
    this.onWrite(ref);
    await this.#plugin.bindings.set({ plugin: ref.plugin, command: ref.command }, inputs);
  }
  /** A user's reset of one command (or of every override). */
  async reset(ref?: BindingRef) {
    if (ref) this.onWrite(ref);
    await this.#plugin.bindings.reset(ref && { plugin: ref.plugin, command: ref.command });
  }

  /**
   * Move `input` to `target` (or nowhere): first take it from every other
   * claimer (`remove` decides which of a claimer's inputs go), then give it
   * to `target`. When the second step fails the first is undone. Resolves
   * after the list has been read again.
   */
  async move(input: string, claimers: BindingInfo[], target: BindingInfo | null, remove: (each: string) => boolean) {
    const undo: (() => Promise<void>)[] = [];
    try {
      for (const owner of claimers) {
        if (target && owner.plugin === target.plugin && owner.command === target.command) continue;
        const before = { overridden: owner.overridden, current: [...owner.current] };
        const next = owner.current.filter((each) => !remove(each));
        // Back where its defaults put it: drop the override instead.
        if (sameSet(next, owner.defaults)) await this.reset(owner);
        else await this.write(owner, next);
        undo.push(() => (before.overridden ? this.write(owner, before.current) : this.reset(owner)));
      }
      if (target && !target.current.includes(input)) {
        const next = [...target.current, input];
        await (sameSet(next, target.defaults) && target.overridden ? this.reset(target) : this.write(target, next));
      }
    } catch (error) {
      for (const step of undo.reverse()) await step().catch(() => {});
      throw error;
    } finally {
      await this.reload().catch(() => {});
    }
  }
}
