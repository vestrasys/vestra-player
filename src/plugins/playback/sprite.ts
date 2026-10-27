/**
 * I07 seek-bar preview (0.2.0): the plan of a source's keyframe sprite.
 *
 * A source gets up to `MAX_CELLS` cells of 160×90, one per equal span of the
 * duration (spans of at least `MIN_SPAN` seconds). Cells are packed into JPEG
 * sheets of 10×10 (the layout Bilibili's own preview sheets use), so 200 cells
 * are two sheets. The bar crops the cell under the pointer itself
 * (`source-clip-*`), so hover and drag need no round trip to the plugin.
 */
export const CELL_WIDTH = 160;
export const CELL_HEIGHT = 90;
export const COLUMNS = 10;
export const PER_SHEET = 100;
export const MAX_CELLS = 200;
/** Seconds: cells never get closer than this (a 20 s clip has 10 cells). */
export const MIN_SPAN = 2;
/** Bytes of one cell as ffmpeg writes it (`rgb24`). */
export const CELL_BYTES = CELL_WIDTH * CELL_HEIGHT * 3;

/** How many keyframe cells a source of `duration` seconds gets (0 without a duration). */
export function cellCount(duration: number | null): number {
  if (duration === null || !Number.isFinite(duration) || duration <= 0) return 0;
  return Math.max(1, Math.min(MAX_CELLS, Math.floor(duration / MIN_SPAN)));
}

// ------------------------------------------------------------ exact mode (0.2.2)

/**
 * Seconds between exact cells: 0.5 s, or duration/200 when that is coarser.
 * Half a second is finer than a pointer on the seek line can aim (a 30 s file
 * gets 60 cells, 10–25 px each on a usual bar), and density costs nothing in
 * this mode: the pass decodes every frame anyway, only the chosen ones are
 * scaled. The 200-cell cap keeps the two-sheet layout the bar already has.
 */
export const EXACT_SPAN = 0.5;
/**
 * The budget: an exact pass may cost about this many seconds of decoding.
 * It is what the keyframe sprite of a long file already costs today (200
 * cells at 65–90 ms each, measured 1080p).
 */
export const EXACT_BUDGET_S = 15;
/**
 * Software decoding costs about this per pixel (measured 2.8–4.2 ns on
 * 1080p30/60 H.264 and 4K60 10-bit HEVC with ffmpeg n9 on a Ryzen 7940HS;
 * two parts at a time make the wall time about half). So 15 s buys 3.75
 * billion pixels: 1080p30 for 60 s, 1080p60 for 30 s, 720p30 for 2.2 min,
 * 4K60 for 7.5 s.
 *
 * Software only, on purpose: ffmpeg's hardware decoding (`-hwaccel` with the
 * player's own method) is ten times cheaper in CPU, but it shares the one
 * video engine the player is decoding on. Measured in the real player on 4K60
 * HEVC (d3d11va-copy, AMD 780M): two d3d11va ffmpeg took 40 s instead of the
 * 8–10 s they take alone, and playback fell to 18–31 frames a second while
 * they ran. The headless probe (one ffmpeg, no window) had not shown it.
 */
export const DECODE_NS_PER_PIXEL = 4;
/**
 * The estimate is for this machine's class. A part that takes more than
 * `SLOW_FACTOR` times its estimate (plus `SLOW_ALLOWANCE_MS` for the process
 * start and jitter) says the machine is slower or busier than assumed: the
 * pass ends and the file gets the keyframe sprite.
 */
export const SLOW_FACTOR = 2;
export const SLOW_ALLOWANCE_MS = 1000;

export type Mode = "keyframe" | "exact";
/** The video facts the plan needs (`stats().video`, each `null` when the player has none). */
export type VideoFacts = {
  width: number | null;
  height: number | null;
  fps: number | null;
};
/** How a source's sprite is made. */
export type Plan = { mode: Mode; count: number };

/** Pixels decoded in `seconds` of the video, `null` when the facts are missing. */
export function decodeWork(seconds: number, video: VideoFacts): number | null {
  const { width, height, fps } = video;
  if (!width || !height || !fps || width < 0 || height < 0 || fps <= 0) return null;
  return width * height * fps * seconds;
}

/** Seconds decoding `work` pixels is estimated to take. */
export const estimatedSeconds = (work: number) => (work * DECODE_NS_PER_PIXEL) / 1e9;

/** Exact cells for `duration` seconds: every `EXACT_SPAN`, never finer than a frame (`fps`), at most `MAX_CELLS`. */
export const exactCellCount = (duration: number, fps: number) =>
  Math.max(1, Math.min(MAX_CELLS, Math.floor(duration / Math.max(EXACT_SPAN, 1 / fps))));

/**
 * Exact when decoding the whole file is estimated within the budget: every
 * cell is then the real frame at its time, whatever the keyframe interval.
 * Otherwise (or without video facts) the keyframe sprite as before.
 */
export function plan(duration: number, video: VideoFacts | null): Plan {
  const keyframe: Plan = { mode: "keyframe", count: cellCount(duration) };
  if (!video) return keyframe;
  const work = decodeWork(duration, video);
  if (work === null || estimatedSeconds(work) > EXACT_BUDGET_S) return keyframe;
  return { mode: "exact", count: exactCellCount(duration, video.fps!) };
}

/** The time an exact cell shows (the frame on screen then): the middle of its span. */
export const exactCellTime = (index: number, count: number, duration: number) => ((index + 0.5) * duration) / count;

/** One ffmpeg run of exact mode: cells `first` … `first + n - 1`, decoded from the keyframe at `start` seconds. */
export type Part = { first: number; n: number; start: number };

/** Seconds of video a part covers at least (keyframes are grouped until then). */
export const PART_S = 8;

/** Milliseconds a part is estimated to take: its cells' share of the video, one thread. */
export function partEstimateMs(part: Part, count: number, duration: number, video: VideoFacts): number {
  return estimatedSeconds(decodeWork((part.n * duration) / count, video) ?? 0) * 1000;
}

/** Whether a part that took `wallMs` against `estimateMs` says the machine cannot afford the pass. */
export const tooSlow = (wallMs: number, estimateMs: number) => wallMs > SLOW_FACTOR * estimateMs + SLOW_ALLOWANCE_MS;

/**
 * The parts of an exact pass, aligned on the file's keyframes (`keyframes`:
 * their times, from ffprobe) so no part decodes frames another part needs:
 * each begins at a keyframe and takes the cells up to the next part's. Parts
 * come coarse to fine, like cells. Without keyframes: one part from 0.
 */
export function partition(count: number, duration: number, keyframes: readonly number[]): Part[] {
  const bounds = [0];
  for (const k of [...keyframes].sort((a, b) => a - b)) if (k - bounds[bounds.length - 1]! >= PART_S) bounds.push(k);
  bounds.push(Number.POSITIVE_INFINITY);
  const parts: Part[] = [];
  const span = duration / count;
  // The cells whose time is at or after a bound: (i + 0.5) * span >= bound.
  const cellFrom = (bound: number) => Math.min(count, Math.max(0, Math.ceil(bound / span - 0.5)));
  for (let b = 0; b + 1 < bounds.length; b++) {
    const first = cellFrom(bounds[b]!);
    const end = cellFrom(bounds[b + 1]!);
    if (end > first) parts.push({ first, n: end - first, start: bounds[b]! });
  }
  return coarseToFine(parts.length).map((i) => parts[i]!);
}

/**
 * Where ffmpeg seeks for a cell: the middle of its span, at least half a
 * second inside the end. ffmpeg then takes the keyframe at or before it.
 */
export function cellTime(index: number, count: number, duration: number): number {
  const middle = ((index + 0.5) * duration) / count;
  return Math.max(0, Math.min(middle, duration - 0.5));
}

/** The cell under a fraction of the seek line (what the bar computes in Slint). */
export const cellAt = (fraction: number, count: number) =>
  Math.min(count - 1, Math.max(0, Math.floor(fraction * count)));

/**
 * The order cells are made in: coarse to fine. First every 64th cell, then the
 * ones between at every 32nd, …, down to every cell, so a partly made sprite
 * already covers the whole seek line evenly.
 */
export function coarseToFine(count: number): number[] {
  const order: number[] = [];
  const taken = new Uint8Array(count);
  let stride = 1;
  while (stride * 2 < count) stride *= 2;
  for (; stride >= 1; stride /= 2)
    for (let index = 0; index < count; index += stride)
      if (!taken[index]) {
        taken[index] = 1;
        order.push(index);
      }
  return order;
}

/** Sheets for `count` cells. */
export const sheetCount = (count: number) => Math.ceil(count / PER_SHEET);

/** Pixel size of sheet `sheet`: 10 columns, as many rows of 90 as its cells need. */
export function sheetSize(sheet: number, count: number): { width: number; height: number } {
  const cells = Math.min(PER_SHEET, count - sheet * PER_SHEET);
  return { width: COLUMNS * CELL_WIDTH, height: Math.ceil(cells / COLUMNS) * CELL_HEIGHT };
}

/** Where a cell is: its sheet and its top-left pixel there. */
export function cellPlace(index: number): { sheet: number; x: number; y: number } {
  const inSheet = index % PER_SHEET;
  return {
    sheet: Math.floor(index / PER_SHEET),
    x: (inSheet % COLUMNS) * CELL_WIDTH,
    y: Math.floor(inSheet / COLUMNS) * CELL_HEIGHT,
  };
}

/** Cell `slot` of a strip of `count` cells side by side (as one ffmpeg run writes them). */
export function stripCell(strip: Uint8Array, slot: number, count: number): Uint8Array {
  const row = CELL_WIDTH * 3;
  const cell = new Uint8Array(CELL_BYTES);
  for (let line = 0; line < CELL_HEIGHT; line++) {
    const from = (line * count + slot) * row;
    cell.set(strip.subarray(from, from + row), line * row);
  }
  return cell;
}

/** Copy one cell (`CELL_BYTES` of rgb24) into its sheet's pixels. */
export function placeCell(sheet: Uint8Array, sheetWidth: number, index: number, cell: Uint8Array) {
  const { x, y } = cellPlace(index);
  const row = CELL_WIDTH * 3;
  for (let line = 0; line < CELL_HEIGHT; line++)
    sheet.set(cell.subarray(line * row, (line + 1) * row), ((y + line) * sheetWidth + x) * 3);
}

/**
 * For each cell, the made cell the bar shows there: itself once made, else the
 * nearest made one (the earlier one on a tie); -1 while none is made.
 */
export function nearestMade(made: ArrayLike<number | boolean>): number[] {
  const count = made.length;
  const map = new Array<number>(count).fill(-1);
  let last = -1;
  for (let i = 0; i < count; i++) {
    if (made[i]) last = i;
    map[i] = last;
  }
  let next = -1;
  for (let i = count - 1; i >= 0; i--) {
    if (made[i]) next = i;
    if (next >= 0 && (map[i] < 0 || next - i < i - map[i])) map[i] = next;
  }
  return map;
}
