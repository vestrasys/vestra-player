import type { Player, Scope, Window } from "../../sdk/plugin/index.ts";

type Labels = { windowTitle: string; groupTitle: string; actionLabel: string };

/**
 * A small tool window with one input (跳转到…, custom speed). It belongs to the
 * source that was current when it opened: another source hides it and a late
 * confirm is refused. Within that source the action applies to the current state.
 */
export class Prompt {
  readonly window: Window;
  message = "";
  #sourceId: number | null = null;
  readonly #player: Player;
  readonly #initial: () => string;

  constructor(
    scope: Scope,
    id: string,
    labels: Labels,
    initial: () => string,
    submit: (value: string) => Promise<void> | void,
  ) {
    this.#player = scope.player;
    this.#initial = initial;
    this.window = scope.windows.create("ui/prompt.slint", { id });
    this.window.bind(() => ({ ...labels, message: this.message }));
    this.window.on("cancel", () => this.hide());
    this.window.on("confirm", async (value: string) => {
      if (this.#sourceId !== this.#player.state.sourceId) {
        this.message = "来源已变化，请重新打开";
        return;
      }
      try {
        await submit(value);
      } catch (error) {
        this.message = error instanceof Error ? error.message : String(error);
        return;
      }
      await this.hide();
    });
  }

  async show() {
    this.#sourceId = this.#player.state.sourceId;
    this.message = "";
    this.window.set({ value: this.#initial() });
    await this.window.show();
  }
  async hide() {
    this.#sourceId = null;
    this.message = "";
    if (this.window.opened) await this.window.hide();
  }
  /** Another source replaced the one this window was opened for. */
  invalidate() {
    if (this.#sourceId !== null && this.#sourceId !== this.#player.state.sourceId) return this.hide();
  }
}
