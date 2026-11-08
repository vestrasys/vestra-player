import { MAX_RATE, MIN_RATE } from "./controller.ts";

/**
 * The Playback panel's speed ruler (menu redesign "Speed B", mockup 12): every preset is a
 * labelled tick and the scale is linear between neighbouring ticks. 1× sits in the centre;
 * the slow half (0.25 → 0.5 → 1) and the fast half (1 → 1.25 → 1.5 → 2 → 4) each space
 * their ticks equally. A position is 0 (the first tick) … 1 (the last).
 */
export const TICKS = [0.25, 0.5, 1, 1.25, 1.5, 2, 4] as const;
/** Where each tick sits (ui/panel.slint draws the ticks at these positions). */
export const TICK_POSITIONS = [0, 0.25, 0.5, 0.625, 0.75, 0.875, 1] as const;
/** Between ticks a drag moves in this step. */
export const DRAG_STEP = 0.05;

const LAST = TICKS.length - 1;

/** The speed limits, as the controller checks them. */
export function clampRate(rate: number): number {
  return Math.min(MAX_RATE, Math.max(MIN_RATE, rate));
}

/** The speed at a position (clamped to 0–1), unrounded. */
export function rateAt(position: number): number {
  const p = Math.min(1, Math.max(0, position));
  let index = 0;
  while (index < LAST - 1 && p > TICK_POSITIONS[index + 1]) index++;
  const [p0, p1] = [TICK_POSITIONS[index]!, TICK_POSITIONS[index + 1]!];
  const [r0, r1] = [TICKS[index]!, TICKS[index + 1]!];
  return r0 + ((p - p0) / (p1 - p0)) * (r1 - r0);
}

/** Where a speed sits (clamped to the limits). */
export function positionOf(rate: number): number {
  const r = clampRate(rate);
  let index = 0;
  while (index < LAST - 1 && r > TICKS[index + 1]) index++;
  const [p0, p1] = [TICK_POSITIONS[index]!, TICK_POSITIONS[index + 1]!];
  const [r0, r1] = [TICKS[index]!, TICKS[index + 1]!];
  return p0 + ((r - r0) / (r1 - r0)) * (p1 - p0);
}

/** The tick that is this speed, -1 for none. */
export function tickOf(rate: number): number {
  return TICKS.findIndex((tick) => Math.abs(tick - rate) < 1e-6);
}

/**
 * The speed a drag to `position` sets: the nearest tick when the position is within `snap`
 * of it (a fraction of the track: the panel turns a few pixels into it), else the speed
 * there in 0.05 steps, within the limits.
 */
export function dragRate(position: number, snap: number): number {
  const p = Math.min(1, Math.max(0, Number.isFinite(position) ? position : 0));
  let nearest = 0;
  for (let index = 1; index <= LAST; index++)
    if (Math.abs(p - TICK_POSITIONS[index]) < Math.abs(p - TICK_POSITIONS[nearest])) nearest = index;
  if (Math.abs(p - TICK_POSITIONS[nearest]) <= Math.max(0, Number.isFinite(snap) ? snap : 0)) return TICKS[nearest];
  const stepped = Math.round(rateAt(p) / DRAG_STEP) * DRAG_STEP;
  return clampRate(Number(stepped.toFixed(2)));
}
