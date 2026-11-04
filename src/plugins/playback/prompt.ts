import type { Player, Scope, Window } from "@vestra-player/sdk";
import { errorMessage, Failure, type FailureKind } from "./texts.ts";

/** Which window: Jump to Time… or the custom speed; `ui/prompt.slint` has the texts. */
export type PromptKind = "jump" | "rate";

/**
 * The message line under the input: `stale` (the source changed), one of the
 * failures, or `other` (the player's message in `detail`); "" for none.
 */
export type Notice = "" | "stale" | FailureKind | "other";

/**
 * A small tool window with one input (Jump to Time…, custom speed). It belongs to the
 * source that was current when it opened: another source hides it and a late
 * confirm is refused. Within that source the action applies to the current state.
 */
export class Prompt {
  readonly promptWindow: Window;
  notice: Notice = "";
  detail = "";
  #sourceId: number | null = null;
  readonly #player: Player;
  readonly #initial: () => string;

  constructor(
    scope: Scope,
    id: string,
    kind: PromptKind,
    initial: () => string,
    submit: (value: string) => Promise<void> | void,
  ) {
    this.#player = scope.player;
    this.#initial = initial;
    this.promptWindow = scope.windows.create("ui/prompt.slint", { id });
    this.promptWindow.bind(() => ({ kind, notice: this.notice, detail: this.detail }));
    this.promptWindow.on("cancel", () => this.hide());
    this.promptWindow.on("confirm", async (value: string) => {
      if (this.#sourceId !== this.#player.state.sourceId) {
        this.#tell("stale");
        return;
      }
      try {
        await submit(value);
      } catch (error) {
        if (error instanceof Failure) this.#tell(error.kind, error.detail);
        else this.#tell("other", errorMessage(error));
        return;
      }
      await this.hide();
    });
  }

  #tell(notice: Notice, detail = "") {
    this.notice = notice;
    this.detail = detail;
  }

  async show() {
    this.#sourceId = this.#player.state.sourceId;
    this.#tell("");
    this.promptWindow.set({ value: this.#initial() });
    await this.promptWindow.show();
  }
  async hide() {
    this.#sourceId = null;
    this.#tell("");
    if (this.promptWindow.opened) await this.promptWindow.hide();
  }
  /** Another source replaced the one this window was opened for. */
  invalidate() {
    if (this.#sourceId !== null && this.#sourceId !== this.#player.state.sourceId) return this.hide();
  }
}
