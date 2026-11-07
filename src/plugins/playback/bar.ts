import type { HostAction, Json, Panel, Plugin, TimelineMark, TimelineRange } from "@vestra-player/sdk";
import type { Playback } from "./controller.ts";
import { time } from "./format.ts";
import type { OutputMemory } from "./output-memory.ts";
import type { SettingsStore } from "./settings.ts";
import { sourceFailure } from "./texts.ts";
import type { Preview, Thumbnails } from "./thumbs.ts";
import { barTips, Keys } from "./tips.ts";

/**
 * The marks the bar shows, at most: within the rows and values of one property
 * value the player takes (`UI_VALUE_LIMITS`; `marks` is 1 + 3 values a row),
 * and what the bar's lower-bound search (`bar.slint`) closes in 13 halvings.
 * More marks than that are sampled (`sampled`): the user's bookmarks first.
 */
export const MAX_MARKS = 4096;
/** The bar's span reports while a resize goes on: the last one wins after this pause. */
export const SPAN_DEBOUNCE_MS = 100;
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
/**
 * A kind's tick commands, at most: a safety net under the player's 64 KiB per
 * string. It is not reached in practice: one tick per mark (≤ 4096) of at most
 * 10 bytes while the line is under 100000 physical px is 40 KiB.
 */
export const TICK_BYTES = 60 * 1024;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/** What JSON writes as a 6-byte `\uXXXX` escape: control characters and lone surrogates. */
const SIX_BYTES = /[\u0000-\u001f]|[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g;
/** An upper bound of `text`'s bytes inside a JSON string (without the quotes). */
export const jsonBytes = (text: string) => 3 * text.length + 3 * (text.match(SIX_BYTES)?.length ?? 0);

/** `count` of `indices` picked evenly (the first and the last kept), in order. */
function evenly(indices: readonly number[], count: number): number[] {
  if (indices.length <= count) return [...indices];
  if (count <= 0) return [];
  if (count === 1) return [indices[0]!];
  const step = (indices.length - 1) / (count - 1);
  return Array.from({ length: count }, (_, i) => indices[Math.round(i * step)]!);
}

/**
 * At most MAX_MARKS of the time-sorted marks: all of them, or else the user's
 * bookmarks first (all of them, or MAX_MARKS picked evenly if they alone are
 * more) and the other kinds picked evenly by index into the room left (the
 * first and the last kept), so the shown marks still run across the whole
 * duration (dropping the tail would leave the end of a long file bare), in
 * proportion to where the marks are, and stay in time order for the bar's
 * search.
 */
export function sampled(marks: readonly TimelineMark[]): readonly TimelineMark[] {
  if (marks.length <= MAX_MARKS) return marks;
  const bookmarks: number[] = [];
  const others: number[] = [];
  marks.forEach((mark, i) => (mark.kind === "bookmark" ? bookmarks : others).push(i));
  const kept = evenly(bookmarks, MAX_MARKS);
  const picked = [...kept, ...evenly(others, MAX_MARKS - kept.length)].sort((a, b) => a - b);
  return picked.map((i) => marks[i]!);
}

export interface SeekMarks {
  /** Sorted by time: `at` the fraction of the duration (the bar's hover search), `tip` "Title  01:23". */
  rows: Json[];
  /** One Path's commands per kind: a 1 physical px tick `M<x> 0V1` per distinct physical pixel column. */
  chapterTicks: string;
  bookmarkTicks: string;
  markerTicks: string;
  /** The seek line's span in physical px the ticks were built for: the Paths' `viewbox-width` (0: none yet). */
  tickSpan: number;
}

const NO_MARKS: SeekMarks = { rows: [], chapterTicks: "", bookmarkTicks: "", markerTicks: "", tickSpan: 0 };

/**
 * B22: `timeline.marks` as the seek line shows them, sorted by time (as the
 * player merges them), at most MAX_MARKS: the hover rows, and the ticks as
 * one Path per kind (no element per mark).
 *
 * `span` is the seek line's width less its two 4px insets in whole physical
 * pixels (`floor(span / 1phx)`), as the bar reports it (`spanChanged`; 0
 * before the first report: no ticks yet). A tick fills exactly one physical
 * pixel column at any scale factor: x = `floor(at * span)` in a viewbox
 * `span` wide over a Path `span + 1` physical px wide whose left edge the bar
 * puts on a physical pixel, so the scale is 1 physical px per unit; Slint
 * insets the path by half the 1phx stroke (`Path::fitted_path_events`), so
 * the stroke is centred on x + 0.5 physical px: one whole column, never two
 * half-transparent ones (`bar.slint`, MarkTicks).
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

export interface SeekRanges {
  /** One Path's commands per style: a filled rectangle `M<x0> 0H<x1>V1H<x0>Z` per run of physical px columns. */
  selectionRanges: string;
  regionRanges: string;
}

const NO_RANGES: SeekRanges = { selectionRanges: "", regionRanges: "" };

/**
 * `timeline.ranges` (player 0.7.6) as the seek line shows them: one filled
 * Path per style (no element per range), in the ticks' view box (`span`
 * physical px wide, 1 high). A range covers the columns `floor(start * span)`
 * up to `ceil(end * span)` (at least one, inside the line); the ranges of a
 * style that overlap or touch are drawn as one rectangle, so a column is
 * filled once whatever the overlap. Nothing without a known duration or
 * before the bar reports its span (as the ticks).
 */
export function seekRanges(ranges: readonly TimelineRange[], duration: number | null, span = 0): SeekRanges {
  if (!duration || duration <= 0 || !(span > 0) || ranges.length === 0) return NO_RANGES;
  const runs: Record<string, [number, number][]> = { selection: [], region: [] };
  for (const range of [...ranges].sort((a, b) => a.start - b.start)) {
    const style = runs[range.style];
    if (!style) continue; // a style a later player may add: not drawn
    const x0 = Math.min(Math.floor(clamp01(range.start / duration) * span), span - 1);
    const x1 = Math.max(x0 + 1, Math.min(span, Math.ceil(clamp01(range.end / duration) * span)));
    const last = style.at(-1);
    if (last && x0 <= last[1]) last[1] = Math.max(last[1], x1);
    else style.push([x0, x1]);
  }
  const path = (list: [number, number][]) => list.map(([x0, x1]) => `M${x0} 0H${x1}V1H${x0}Z`).join("");
  return { selectionRanges: path(runs.selection!), regionRanges: path(runs.region!) };
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
  "previous",
  "next",
  "mute",
  // UI-BAR-05 (0.2.4): toggles the main window's fullscreen, as the Toggle Fullscreen key (Escape by default) does.
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
  // B22: the seek line's span in physical px, which the bar reports when its
  // width (or the scale factor) changes. The first report is taken at once
  // (there are no ticks before it); later ones wait for a pause of
  // SPAN_DEBOUNCE_MS, the last winning, so a window resize does not rebuild
  // and send the ticks every frame (they stretch with the line meanwhile).
  let span = 0;
  let pending: ReturnType<typeof setTimeout> | null = null;
  const takeSpan = (px: number) => {
    if (px === span) return;
    span = px;
    scope.update();
  };
  bar.on("spanChanged", (reported: number) => {
    const px = Math.floor(reported);
    if (!(px > 0)) return;
    if (pending !== null) clearTimeout(pending);
    pending = null;
    if (span === 0) return takeSpan(px);
    pending = setTimeout(() => {
      pending = null;
      takeSpan(px);
    }, SPAN_DEBOUNCE_MS);
  });
  scope.onDispose(() => {
    if (pending !== null) clearTimeout(pending);
  });
  let drawn: { marks: readonly TimelineMark[]; duration: number | null; span: number; shown: SeekMarks } | null = null;
  const marks = (duration: number | null) => {
    const current = scope.timeline.marks;
    if (drawn?.marks !== current || drawn.duration !== duration || drawn.span !== span)
      drawn = { marks: current, duration, span, shown: seekMarks(current, duration, span) };
    return drawn.shown;
  };
  // Timeline ranges, read in the binding: a change of anyone's ranges rebinds.
  type Ranged = { ranges: readonly TimelineRange[]; duration: number | null; span: number; shown: SeekRanges };
  let ranged: Ranged | null = null;
  const ranges = (duration: number | null) => {
    const current = scope.timeline.ranges;
    if (ranged?.ranges !== current || ranged.duration !== duration || ranged.span !== span)
      ranged = { ranges: current, duration, span, shown: seekRanges(current, duration, span) };
    return ranged.shown;
  };
  const p = playback;
  let volumeDragging = false;
  // I07: the pointer is on the seek line (hover or drag): the preview's sheets go out.
  let pointing = false;
  bar.on("pointing", (on: boolean) => {
    pointing = on;
    thumbnails.pointing(on); // 0.2.3: new sheets are handed over less often meanwhile
    locks();
    scope.update();
  });
  bar.bind(() => {
    const state = player.state;
    const position = p.draft ?? state.position;
    const volume = p.volumeDraft ?? state.volume;
    const remaining = settings.value.remainingTime;
    const left = remaining && position !== null && state.duration !== null;
    const shown = marks(state.duration);
    const spans = ranges(state.duration);
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
      // Timeline ranges (player 0.7.6), in the ticks' view box: one Path per style.
      selectionRanges: spans.selectionRanges,
      regionRanges: spans.regionRanges,
      hasLoop: Boolean(state.loop && state.duration),
      loopA: state.loop && state.duration ? clamp01(state.loop.a / state.duration) : 0,
      loopB: state.loop && state.duration ? clamp01(state.loop.b / state.duration) : 0,
      canPause: p.canToggle(),
      playing: state.phase === "ready" && !state.paused,
      canStop: p.can("close"),
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
      ...barTips(keys, { volume }),
    };
  });
  // 0.2.11: the pointer on the seek band holds the bar too (`popupOpen`: its time
  // tip and preview are up). The band reaches 4px above the bar, over the video,
  // where the player sees the pointer over neither, so without this the
  // fullscreen bar would start hiding under it.
  const locks = () =>
    bar.setInteraction({ focused: false, dragging: p.dragging || volumeDragging, popupOpen: pointing });

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
