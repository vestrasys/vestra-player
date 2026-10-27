import type { HostAction, Json, Panel, Plugin, TimelineMark } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { OutputMemory } from "./output-memory.ts";
import type { SettingsStore } from "./settings.ts";
import { sourceFailure } from "./texts.ts";
import type { Preview, Thumbnails } from "./thumbs.ts";
import { barTips, Keys } from "./tips.ts";

/** The bar finds the hovered mark with a fixed-depth binary search (`bar.slint`): 2^13 marks at most. */
export const MAX_MARKS = 8192;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/**
 * B22: `timeline.marks` as the seek line draws them, sorted by time (as the
 * player merges them): `at` is the fraction of the duration, `clock` the
 * mark's time for the hover tip "Title  01:23".
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

/**
 * I07: the preview box (`hoverThumbnail`) and the sprite the bar crops the
 * pointer's cell from: `previewMap[cell]` is the made cell to show there (-1:
 * none yet); cell `c` is on sheet `c / 100`, column `c % 10`, row `c % 100 / 10`.
 * Sheets are sent only once they exist (an image property cannot be emptied),
 * and only while the pointer is on the seek line (`pointing`, hover or drag):
 * the player decodes each sheet it gets on its UI thread, which playback
 * should not pay for once a second while nobody looks. Meanwhile the count is
 * 0, so the bar never pairs a newer map with an older sheet.
 */
export function previewProps(active: boolean, preview: Preview | null, pointing: boolean) {
  const shown = active && pointing ? preview : null;
  return {
    hoverThumbnail: active,
    previewCount: shown?.count ?? 0,
    previewMap: shown?.map ?? [],
    ...(shown?.sheets[0] ? { previewSheetA: shown.sheets[0] } : {}),
    ...(shown?.sheets[1] ? { previewSheetB: shown.sheets[1] } : {}),
  };
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
  // UI-BAR-05 (0.2.4): toggles the main window's fullscreen, as Enter / F11 do.
  "fullscreen",
]);

/**
 * Project the playback state onto `ui/bar.slint` and route its callbacks. The
 * buttons' enabled flags still come from `player.state`; their actions are the
 * core's (`host.run`), except Open (this plugin's) and the volume slider, which
 * needs continuous values (`player.setOutput`, the same rules as volume ±).
 */
export function wireBar(
  bar: Panel,
  scope: Plugin,
  playback: Playback,
  settings: SettingsStore,
  thumbnails: Thumbnails,
  output: OutputMemory,
) {
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
  // I07: the pointer is on the seek line (hover or drag): the preview's sheets go out.
  let pointing = false;
  bar.on("pointing", (on: boolean) => {
    pointing = on;
    thumbnails.pointing(on); // 0.2.3: new sheets are handed over less often meanwhile
    scope.update();
  });
  bar.bind(() => {
    const state = player.state;
    const position = p.draft ?? state.position;
    const volume = p.volumeDraft ?? state.volume;
    const remaining = settings.value.remainingTime;
    const left = remaining && position !== null && state.duration !== null;
    const slot = sidebar.state;
    const sidebarHidden = !!slot?.preferred && !slot.effective && slot.reason === "space";
    return {
      elapsedLabel: left ? `-${time(state.duration! - position!)}` : time(position),
      totalLabel: time(state.duration),
      progress: state.duration && position !== null ? position / state.duration : 0,
      canSeek: p.canSeek(),
      // I06: the bar computes the hover time from these, without a round trip.
      duration: state.duration ?? 0,
      hoverTime: settings.value.hoverTime,
      // I07: the picture box, and the sprite the bar crops the pointer's cell from (hover and drag).
      ...previewProps(thumbnails.active(state), thumbnails.preview, pointing),
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
      // G-13b: the slider runs 0–100; a louder volume (set elsewhere) shows at the right end.
      volume: Math.min(volume, 100),
      canMute: !p.muteBusy && p.volumeDraft === null,
      canVolume: !p.muteBusy,
      hasSidebar: slot !== null,
      sidebarOn: slot?.preferred ?? false,
      // G-18: on but not shown because the window is too narrow (fullscreen's collapsed pop-out counts as on).
      sidebarHidden: sidebarHidden,
      // UI-BAR-05: the fullscreen button shows enter or leave.
      fullscreen: scope.window.state.fullscreen,
      // G-14: which time the label shows (the tooltip offers the other).
      remainingTime: remaining,
      ...barTips(keys, { volume, step: settings.value.seekSteps[0] }),
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
      throw sourceFailure(error);
    } finally {
      if (id === "mute") p.muteBusy = false;
    }
  });
  // G-14: the choice is saved (`remainingTime`) and kept across restarts.
  bar.on("toggleTime", () => settings.update((s) => void (s.remainingTime = !s.remainingTime)));
  bar.on("toggleSidebar", () => sidebar.toggle());
  bar.on("volumeChanged", (value: number) => {
    if (!volumeDragging) {
      volumeDragging = true;
      output.hold(true);
      locks();
    }
    p.setVolume(value);
  });
  bar.on("volumeReleased", (value: number) => {
    volumeDragging = false;
    locks();
    p.setVolume(value, true);
    output.hold(false);
  });
  // Pointer down → dragging(true) + seekChanged…; up → seekChanged + dragging(false).
  bar.on("dragging", (dragging: boolean) => {
    p.dragging = dragging;
    thumbnails.hold(dragging); // I07: the drag's seeks get the CPU

    // 0.2.3: the gesture's seek mode is the "Jump to keyframes only" switch as
    // it is at the press (the same switch as the arrow keys); the bar does not
    // auto-hide while dragging.
    if (dragging) p.startDrag(settings.value.seekKeyframes);
    else {
      if (p.draft !== null && !p.blockedGesture) p.notify("Seek: {} / {}", p.positionArgs(p.draft));
      p.endDrag();
    }
    locks();
  });
  bar.on("seekChanged", (fraction: number) => {
    if (p.canSeek() && !p.blockedGesture) p.gestureSeek(fraction * player.state.duration!);
  });
  // Unsent points are dropped; a seek the host already accepted is not undone.
  bar.on("seekCancelled", async () => {
    thumbnails.hold(false);
    p.clearSeek();
    p.blockedGesture = true;
    p.dragging = false;
    locks();
    await player.refresh();
  });
}
