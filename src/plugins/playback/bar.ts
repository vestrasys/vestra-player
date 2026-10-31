import type { HostAction, Json, Panel, Plugin, TimelineMark } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { OutputMemory } from "./output-memory.ts";
import type { SettingsStore } from "./settings.ts";
import { sourceFailure } from "./texts.ts";
import type { Preview, Thumbnails } from "./thumbs.ts";
import { barTips, Keys } from "./tips.ts";

/**
 * The marks the bar shows: the rows of one property value the player takes
 * (`UI_VALUE_LIMITS.arrayRows`; `marks` is 1 + 3 values a row, under its 16384).
 * The bar's lower-bound search (`bar.slint`) takes 13 halvings for 4096 rows.
 * More marks than that are sampled evenly in time order (`sampled`).
 */
export const MAX_MARKS = 4096;
/**
 * The hover tips' text, at most (an upper bound of the JSON the player gets:
 * 3 bytes a UTF-16 unit covers UTF-8 and the 2-byte escapes `\"` and `\\`, but
 * a control character or a lone surrogate is written as a 6-byte `\u00XX` /
 * `\udXXX` escape and counts 6, see `jsonBytes`), so the bar's write stays well
 * inside the player's 1 MiB per value and per message whatever the titles;
 * past it a mark's tip is its time alone. 4096 marks with titles of 30
 * characters fit, any script.
 */
export const MARK_TIP_BYTES = 512 * 1024;
/** A kind's tick commands, at most: under the player's 64 KiB per string, whatever the width. */
export const TICK_BYTES = 60 * 1024;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/** What JSON writes as a 6-byte `\uXXXX` escape: control characters and lone surrogates. */
const SIX_BYTES = /[\u0000-\u001f]|[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g;
/** An upper bound of `text`'s bytes inside a JSON string (without the quotes). */
export const jsonBytes = (text: string) => 3 * text.length + 3 * (text.match(SIX_BYTES)?.length ?? 0);

/**
 * At most MAX_MARKS of the time-sorted marks: all of them, or MAX_MARKS
 * picked evenly by index (the first and the last kept), so the shown marks
 * still run across the whole duration (dropping the tail would leave the end
 * of a long file bare), in proportion to where the marks are, and stay sorted
 * for the bar's search.
 */
export function sampled<T>(marks: readonly T[]): readonly T[] {
  if (marks.length <= MAX_MARKS) return marks;
  const step = (marks.length - 1) / (MAX_MARKS - 1);
  return Array.from({ length: MAX_MARKS }, (_, i) => marks[Math.round(i * step)]!);
}

export interface SeekMarks {
  /** Sorted by time: `at` the fraction of the duration (the bar's hover search), `tip` "Title  01:23". */
  rows: Json[];
  /** One Path's commands per kind: a 1px tick `M<x> 0V1` per distinct pixel column. */
  chapterTicks: string;
  bookmarkTicks: string;
  markerTicks: string;
  /** The seek line's span (px) the ticks were built for: the Paths' `viewbox-width` (0: none yet). */
  tickSpan: number;
}

const NO_MARKS: SeekMarks = { rows: [], chapterTicks: "", bookmarkTicks: "", markerTicks: "", tickSpan: 0 };

/**
 * B22: `timeline.marks` as the seek line shows them, sorted by time (as the
 * player merges them), at most MAX_MARKS: the hover rows, and the ticks as
 * one Path per kind (no element per mark).
 *
 * `span` is the seek line's width less its two 4px insets, in px, as the bar
 * reports it (`spanChanged`; 0 before the first report: no ticks yet). A tick
 * covers the whole pixel column the marks had before 0.2.6, `floor(4px + at *
 * span)`: x = `floor(at * span)` in a viewbox `span` wide over a Path `span +
 * 1px` wide at 4px, so the scale is 1; Slint insets the path by half the
 * stroke (`Path::fitted_path_events`), so the 1px stroke is centred on
 * x + 0.5 and fills exactly that column, never two half-transparent ones.
 * Each string stays under TICK_BYTES (one tick per column, ≤ 10 bytes each).
 */
export function seekMarks(marks: readonly TimelineMark[], duration: number | null, span = 0): SeekMarks {
  if (!duration || duration <= 0) return NO_MARKS;
  const rows: Json[] = [];
  const ticks = { chapter: [] as string[], bookmark: [] as string[], marker: [] as string[] };
  const bytes = { chapter: 0, bookmark: 0, marker: 0 };
  const last = { chapter: -1, bookmark: -1, marker: -1 };
  const drawn = span > 0;
  let budget = MARK_TIP_BYTES;
  for (const mark of sampled(marks)) {
    const at = clamp01(mark.time / duration);
    const clock = time(mark.time);
    const title = mark.title ?? "";
    const tip = `${title}  ${clock}`;
    const cost = jsonBytes(tip);
    const titled = title !== "" && cost <= budget;
    if (titled) budget -= cost;
    rows.push({ at, tip: titled ? tip : clock });
    if (!drawn) continue;
    // Marks in time order: a tick in the same column as the kind's previous one adds nothing.
    const x = Math.floor(at * span);
    if (x === last[mark.kind]) continue;
    last[mark.kind] = x;
    const command = `M${x} 0V1`;
    if (bytes[mark.kind] + command.length > TICK_BYTES) continue;
    bytes[mark.kind] += command.length;
    ticks[mark.kind].push(command);
  }
  return {
    rows,
    chapterTicks: ticks.chapter.join(""),
    bookmarkTicks: ticks.bookmark.join(""),
    markerTicks: ticks.marker.join(""),
    tickSpan: drawn ? span : 0,
  };
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
  // B22: the seek line's span in px, which the bar reports when its width changes.
  let span = 0;
  bar.on("spanChanged", (px: number) => {
    if (!(px > 0) || px === span) return;
    span = px;
    scope.update();
  });
  let drawn: { marks: readonly TimelineMark[]; duration: number | null; span: number; shown: SeekMarks } | null = null;
  const marks = (duration: number | null) => {
    const current = scope.timeline.marks;
    if (drawn?.marks !== current || drawn.duration !== duration || drawn.span !== span)
      drawn = { marks: current, duration, span, shown: seekMarks(current, duration, span) };
    return drawn.shown;
  };
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
    const shown = marks(state.duration);
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
      marks: shown.rows,
      chapterTicks: shown.chapterTicks,
      bookmarkTicks: shown.bookmarkTicks,
      markerTicks: shown.markerTicks,
      tickSpan: shown.tickSpan,
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
