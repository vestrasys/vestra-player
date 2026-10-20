import type { Panel, Player, SidebarSlot } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { SettingsStore } from "./settings.ts";

/** Project the playback state onto `ui/bar.slint` and route its callbacks. */
export function wireBar(
  bar: Panel,
  player: Player,
  sidebar: SidebarSlot,
  playback: Playback,
  run: (id: string) => Promise<void>,
  settings: SettingsStore,
) {
  const p = playback;
  /** The time label shows the remaining time instead of the elapsed one (UI-BAR-07). */
  let remaining = false;
  let volumeDragging = false;
  bar.bind(() => {
    const state = player.state;
    const position = p.draft ?? state.position;
    const volume = p.volumeDraft ?? state.volume;
    const left = remaining && position !== null && state.duration !== null;
    const slot = sidebar.state;
    return {
      elapsedLabel: left ? `-${time(state.duration! - position!)}` : time(position),
      totalLabel: time(state.duration),
      progress: state.duration && position !== null ? position / state.duration : 0,
      canSeek: p.canSeek(),
      // I06: the bar computes the hover time from these, without a round trip.
      duration: state.duration ?? 0,
      hoverTime: settings.value.hoverTime,
      canPause: p.canToggle() && !p.pauseBusy,
      playing: state.phase === "ready" && !state.paused,
      canStop: p.can("close"),
      canStep: p.canStep(),
      canPrevious: state.can.previous,
      canNext: state.can.next,
      canOpen: !p.selecting,
      muted: state.muted,
      volume,
      canMute: !p.outputBusy && p.volumeDraft === null,
      canVolume: !p.outputBusy,
      hasSidebar: slot !== null,
      sidebarOn: slot?.preferred ?? false,
    };
  });
  const locks = () =>
    bar.setInteraction({ focused: false, dragging: p.dragging || volumeDragging, popupOpen: false });

  bar.on("action", (id: string) => run(id));
  bar.on("toggleTime", () => {
    remaining = !remaining;
  });
  bar.on("toggleSidebar", () => sidebar.toggle());
  bar.on("volumeChanged", (value: number) => {
    if (!volumeDragging) {
      volumeDragging = true;
      locks();
    }
    p.setVolume(value);
  });
  bar.on("volumeReleased", (value: number) => {
    volumeDragging = false;
    locks();
    p.setVolume(value, true);
  });
  // Pointer down → dragging(true) + seekChanged…; up → seekChanged + dragging(false).
  bar.on("dragging", (dragging: boolean) => {
    p.dragging = dragging;
    if (dragging) p.startDrag(); // the bar does not auto-hide while dragging
    else if (p.draft !== null && !p.blockedGesture) p.notify(`跳转: ${p.positionText(p.draft)}`);
    locks();
  });
  bar.on("seekChanged", (fraction: number) => {
    if (p.canSeek() && !p.blockedGesture) p.seek(fraction * player.state.duration!);
  });
  // Unsent points are dropped; a seek the host already accepted is not undone.
  bar.on("seekCancelled", async () => {
    p.clearSeek();
    p.blockedGesture = true;
    p.dragging = false;
    locks();
    await player.refresh();
  });
}
