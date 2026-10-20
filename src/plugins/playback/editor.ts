import type { Panel, Player, PlayerState } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { parseTime, time } from "./format.ts";

export type EditorKind = "" | "jump" | "rate" | "volume";

/**
 * The bar's inline editor (jump to time, custom rate, volume). Confirming acts
 * only on the source the user saw when the editor opened.
 */
export class Editor {
  kind: EditorKind = "";
  #seen: PlayerState | undefined;
  readonly #playback: Playback;
  readonly #player: Player;
  readonly #bar: Panel;

  constructor(playback: Playback, player: Player, bar: Panel) {
    this.#playback = playback;
    this.#player = player;
    this.#bar = bar;
  }

  open(kind: Exclude<EditorKind, "">) {
    const state = this.#player.state;
    this.kind = kind;
    this.#seen = state;
    this.#playback.error = "";
    const value = kind === "jump" ? time(state.position) : kind === "rate" ? String(state.rate) : String(Math.round(state.volume));
    this.#bar.set({ editorValue: value });
    this.#bar.setInteraction({ focused: true, dragging: false, popupOpen: true });
  }
  cancel() {
    this.kind = "";
    this.#bar.setInteraction({ focused: false, dragging: this.#playback.dragging, popupOpen: false });
  }
  /** Any source change since opening (another file, pause, seek, rate) closes the editor. */
  invalidate() {
    if (this.#seen && !this.#player.isCurrent(this.#seen)) this.kind = "";
  }
  async confirm(value: string) {
    const playback = this.#playback;
    const seen = this.#seen;
    if (!seen || !this.#player.isCurrent(seen)) throw new Error("来源已变化，请重新打开编辑器");
    if (this.kind === "jump") {
      const seconds = parseTime(value);
      if (seconds === null || !playback.canSeek() || seconds > this.#player.state.duration!)
        throw new Error("请输入来源时长范围内的HH:MM:SS.mmm");
      playback.seek(seconds);
    }
    if (this.kind === "rate") await playback.setRate(Number(value), { ifCurrent: seen });
    if (this.kind === "volume") {
      const amount = Number(value);
      if (!value.trim() || !Number.isFinite(amount) || amount < 0 || amount > 200) throw new Error("音量范围为0–200%");
      playback.setVolume(amount);
    }
    this.kind = "";
    this.#bar.setInteraction({ focused: false, dragging: playback.dragging, popupOpen: false });
  }
}
