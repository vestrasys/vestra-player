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

/** How many cells a source of `duration` seconds gets (0 without a duration). */
export function cellCount(duration: number | null): number {
  if (duration === null || !Number.isFinite(duration) || duration <= 0) return 0;
  return Math.max(1, Math.min(MAX_CELLS, Math.floor(duration / MIN_SPAN)));
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
