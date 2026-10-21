import type { HostAction, Json, Panel, Plugin, TimelineMark } from "../../sdk/plugin/index.ts";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { SettingsStore } from "./settings.ts";
import type { Thumbnails } from "./thumbs.ts";
import { barTips, Keys } from "./tips.ts";

/** The bar finds the hovered mark with a fixed-depth binary search (`bar.slint`): 2^13 marks at most. */
export const MAX_MARKS = 8192;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/**
 * B22: `timeline.marks` as the seek line draws them, sorted by time (as the
 * player merges them): `at` is the fraction of the duration, `clock` the
 * mark's time for the hover tip 「标题  01:23」.
 */
export function seekMarks(marks: readonly TimelineMark[], duration: number | null): Json[] {
  if (!duration || duration <= 0) return [];
  return marks.slice(0, MAX_MARKS).map((mark) => ({
    at: clamp01(mark.time / duration),
    kind: mark.kind,
    title: mark.title ?? "",
    clock: time(mark.time),
  }));
}

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
export function wireBar(bar: Panel, scope: Plugin, playback: Playback, settings: SettingsStore, thumbnails: Thumbnails) {
  const { player } = scope;
  // G-07: tooltips name the keys the user has bound (read again when bindings change).
  const keys = new Keys(scope, () => scope.update());
  void keys.refresh();
  let drawn: { marks: readonly TimelineMark[]; duration: number | null; rows: Json[] } | null = null;
  const marks = (duration: number | null) => {
    const current = scope.timeline.marks;
    if (drawn?.marks !== current || drawn.duration !== duration)
      drawn = { marks: current, duration, rows: seekMarks(current, duration) };
    return drawn.rows;
  };
  const sidebar = scope.panels.sidebar;
  const p = playback;
  let volumeDragging = false;
  bar.bind(() => {
    const state = player.state;
    const position = p.draft ?? state.position;
    const volume = p.volumeDraft ?? state.volume;
    const remaining = settings.value.remainingTime;
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
      // I07: the picture box, and the picture once there is one (kept while the next is made).
      hoverThumbnail: thumbnails.active(state),
      hasThumbnail: thumbnails.current !== null,
      ...(thumbnails.current ? { thumbnail: thumbnails.current } : {}),
      // B22: everyone's marks, and the A-B loop (whoever set it).
      marks: marks(state.duration),
      hasLoop: Boolean(state.loop && state.duration),
      loopA: state.loop && state.duration ? clamp01(state.loop.a / state.duration) : 0,
      loopB: state.loop && state.duration ? clamp01(state.loop.b / state.duration) : 0,
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
      ...barTips(keys, {
        playing: state.phase === "ready" && !state.paused,
        muted: state.muted,
        volume,
        remaining,
        step: settings.value.seekSteps[0],
      }),
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
  bar.on("hoverChanged", (fraction: number) => thumbnails.hover(fraction));
  bar.on("hoverEnded", () => thumbnails.hoverEnded());
  // G-14: the choice is saved (`remainingTime`) and kept across restarts.
  bar.on("toggleTime", () => settings.update((s) => void (s.remainingTime = !s.remainingTime)));
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
