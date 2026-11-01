import { ROW_HEIGHT } from "./drop.ts";

/**
 * G-08: list scrolling in logical px (`scroll` = how far the list is scrolled,
 * `height` = the list area's height, `count` = shown rows of `row` px: `ROW_HEIGHT`,
 * or `THUMB_ROW_HEIGHT` while thumbnails are shown).
 */
export const limit = (scroll: number, height: number, count: number, row = ROW_HEIGHT) =>
  Math.max(0, Math.min(scroll, count * row - height));

/** Whether the shown row `index` is wholly in view. */
export function inView(index: number, scroll: number, height: number, row = ROW_HEIGHT) {
  const top = index * row;
  return top >= scroll && top + row <= scroll + height;
}

/**
 * The least scroll that shows row `index` whole (Win32 `LVM_ENSUREVISIBLE`):
 * above the view it becomes the first row, below it the last; in view nothing moves.
 */
export function ensureVisible(index: number, scroll: number, height: number, count: number, row = ROW_HEIGHT) {
  const top = index * row;
  if (top < scroll) return limit(top, height, count, row);
  if (top + row > scroll + height) return limit(top + row - height, height, count, row);
  return scroll;
}

/** The scroll that puts row `index` in the middle of the view (as far as the list allows). */
export function centered(index: number, height: number, count: number, row = ROW_HEIGHT) {
  return limit(index * row + row / 2 - height / 2, height, count, row);
}

/**
 * After the list appears (F6, its tab, detached or attached, the first layout after start-up),
 * it is centred on the playing row, and kept so while its size settles this long; scrolls
 * reported in this time are layout, not the user's (Vestra value).
 */
export const SHOW_SETTLE_MS = 1000;

/** After the user scrolls, the playing row's changes leave the list alone this long (Vestra value). */
export const USER_SCROLL_HOLD_MS = 2000;

/**
 * Thumbnails are made for the rows on screen and this many above and below
 * (a short scroll finds them ready); their durations are read too.
 */
export const THUMB_MARGIN = 3;
/**
 * Pictures already made are sent for this many rows around the view (every other row
 * shows the placeholder): a write then names at most ~60 pictures, each one the player
 * checks, and keeps the view far below its 32 MB image budget (a 192 px picture is
 * about 80 KB decoded, so a few hundred would fill it).
 */
export const THUMB_SEND_MARGIN = 20;
