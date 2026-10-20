import type { Panel, Player } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import type { Editor } from "./editor.ts";
import { time } from "./format.ts";

/** Project the playback state onto `ui/bar.slint` and route its callbacks. */
export function wireBar(bar: Panel, player: Player, playback: Playback, editor: Editor, run: (id: string) => Promise<void>) {
  const p = playback;
  bar.bind(() => {
    const state = player.state;
    const position = p.draft ?? state.position;
    const volume = p.volumeDraft ?? state.volume;
    return {
      sourceLabel: state.source ?? "未打开媒体",
      status: p.status(),
      timeLabel: `${time(position)} / ${time(state.duration)}`,
      progress: state.duration && position !== null ? position / state.duration : 0,
      canSeek: p.canSeek(),
      canPause: p.canToggle() && !p.pauseBusy,
      pauseLabel: p.pauseBusy ? "等待…" : state.phase === "ended" ? "重播" : state.paused ? "播放" : "暂停",
      canOpen: !p.selecting,
      muted: state.muted,
      volume,
      volumeLabel: `${Math.round(volume)}%`,
      rateLabel: `${state.rate}倍`,
      canRate: p.can("rate") && !p.rateBusy,
      canMute: !p.outputBusy && p.volumeDraft === null,
      canVolume: !p.outputBusy,
      editor: editor.kind,
      canRetry: state.phase === "error" && state.source !== null,
    };
  });
  const locks = (focused: boolean) =>
    bar.setInteraction({ focused, dragging: p.dragging, popupOpen: editor.kind !== "" });

  bar.on("action", (id: string) => run(id));
  bar.on("volumeChanged", (value: number) => p.setVolume(value));
  bar.on("volumeReleased", (value: number) => p.setVolume(value));
  // Pointer down → dragging(true) + seekChanged…; up → seekChanged + dragging(false).
  bar.on("dragging", (dragging: boolean) => {
    p.dragging = dragging;
    if (dragging) p.startDrag(); // the bar does not auto-hide while dragging
    locks(false);
  });
  bar.on("seekChanged", (fraction: number) => {
    if (p.canSeek() && !p.blockedGesture) p.seek(fraction * player.state.duration!);
  });
  // Unsent points are dropped; a seek the host already accepted is not undone.
  bar.on("seekCancelled", async () => {
    p.clearSeek();
    p.blockedGesture = true;
    p.dragging = false;
    locks(false);
    await player.refresh();
  });
  bar.on("interaction", (focused: boolean) => locks(focused));
  bar.on("cancelEditor", () => editor.cancel());
  bar.on("confirmEditor", (value: string) => editor.confirm(value));
  bar.on("retry", () => p.retry());
}
