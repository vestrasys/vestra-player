import { ROW_HEIGHT } from "./drop.ts";

/**
 * G-08: list scrolling in logical px (`scroll` = how far the list is scrolled,
 * `height` = the list area's height, `count` = shown rows of `ROW_HEIGHT`).
 */
const limit = (scroll: number, height: number, count: number) =>
  Math.max(0, Math.min(scroll, count * ROW_HEIGHT - height));

/** Whether the shown row `index` is wholly in view. */
export function inView(index: number, scroll: number, height: number) {
  const top = index * ROW_HEIGHT;
  return top >= scroll && top + ROW_HEIGHT <= scroll + height;
}

/**
 * The least scroll that shows row `index` whole (Win32 `LVM_ENSUREVISIBLE`):
 * above the view it becomes the first row, below it the last; in view nothing moves.
 */
export function ensureVisible(index: number, scroll: number, height: number, count: number) {
  const top = index * ROW_HEIGHT;
  if (top < scroll) return limit(top, height, count);
  if (top + ROW_HEIGHT > scroll + height) return limit(top + ROW_HEIGHT - height, height, count);
  return scroll;
}

/** The scroll that puts row `index` in the middle of the view (as far as the list allows). */
export function centered(index: number, height: number, count: number) {
  return limit(index * ROW_HEIGHT + ROW_HEIGHT / 2 - height / 2, height, count);
}

/** After the user scrolls, the playing row's changes leave the list alone this long (Vestra value). */
export const USER_SCROLL_HOLD_MS = 2000;
