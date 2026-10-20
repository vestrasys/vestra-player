import type { HostAction, Panel, Scope } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { SettingsStore } from "./settings.ts";

/** The bar's buttons that run core actions (same behaviour and text as the keys). */
const CORE: ReadonlySet<string> = new Set<HostAction>([
  "play-pause",
  "stop",
  "seek-backward",
  "seek-forward",
  "previous",
  "next",
  "mute",
]);

/**
 * Project the playback state onto `ui/bar.slint` and route its callbacks. The
 * buttons' enabled flags still come from `player.state`; their actions are the
 * core's (`host.run`), except 打开 (this plugin's) and the volume slider, which
 * needs continuous values (`player.setOutput`, the same rules as volume ±).
 */
export function wireBar(bar: Panel, scope: Scope, playback: Playback, settings: SettingsStore) {
  const { player } = scope;
  const sidebar = scope.panels.sidebar;
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
      canPause: p.canToggle(),
      playing: state.phase === "ready" && !state.paused,
      canStop: p.can("close"),
      canStep: p.canStep(),
      canPrevious: state.can.previous,
      canNext: state.can.next,
      canOpen: !p.selecting,
      muted: state.muted,
      volume,
      canMute: !p.muteBusy && p.volumeDraft === null,
      canVolume: !p.muteBusy,
      hasSidebar: slot !== null,
      sidebarOn: slot?.preferred ?? false,
    };
  });
  const locks = () =>
    bar.setInteraction({ focused: false, dragging: p.dragging || volumeDragging, popupOpen: false });

  bar.on("action", async (id: string) => {
    if (id === "open") return p.open();
    if (!CORE.has(id)) return;
    // Mute waits for a volume drag to land, and the slider waits for the mute.
    if (id === "mute") {
      if (p.muteBusy || p.volumeDraft !== null) return;
      p.muteBusy = true;
    }
    try {
      await scope.host.run(id as HostAction);
    } catch (error) {
      // Disabled meanwhile (the button state lags a little): nothing to report.
      if ((error as { code?: string })?.code === "unavailable") return;
      throw new Error(`来源操作失败：${error instanceof Error ? error.message : String(error)}`);
    } finally {
      if (id === "mute") p.muteBusy = false;
    }
  });
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
