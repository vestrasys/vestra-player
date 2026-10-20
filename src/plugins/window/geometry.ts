/** Sizes in logical points (one video pixel = one point at 1×, H03). */
export interface Size {
  width: number;
  height: number;
}

/** H03: the scales of 屏幕 ▶ 0.5倍 … 2倍, with their menu titles. */
export const SCALES: readonly { scale: number; title: string }[] = [
  { scale: 0.5, title: "0.5倍" },
  { scale: 1, title: "1倍" },
  { scale: 1.5, title: "1.5倍" },
  { scale: 2, title: "2倍" },
];

/** The 迷你尺寸 video area fits in this box, keeping the video's aspect ratio. */
export const MINI_BOX: Size = { width: 400, height: 300 };

/** Two sizes are the same when both sides differ by at most one point (H03 6, H04 3). */
export const near = (a: Size, b: Size) => Math.abs(a.width - b.width) <= 1 && Math.abs(a.height - b.height) <= 1;

export const scaled = (video: Size, scale: number): Size => ({
  width: Math.round(video.width * scale),
  height: Math.round(video.height * scale),
});

/** H03 6: the scale whose size the video area has now, if any. */
export const currentScale = (area: Size, video: Size | null) =>
  video ? (SCALES.find(({ scale }) => near(area, scaled(video, scale)))?.scale ?? null) : null;

/** H04: keep the width, height from the video's aspect ratio (the core shrinks both when it does not fit). */
export const fitted = (area: Size, video: Size): Size => ({
  width: Math.round(area.width),
  height: Math.max(1, Math.round((area.width * video.height) / video.width)),
});

/** 迷你尺寸: the largest size of the video's aspect ratio inside `MINI_BOX`. */
export function mini(video: Size): Size {
  const k = Math.min(MINI_BOX.width / video.width, MINI_BOX.height / video.height);
  return { width: Math.max(1, Math.round(video.width * k)), height: Math.max(1, Math.round(video.height * k)) };
}

/** The note after an OSD text when the core changed the requested size. */
export function clampNote(requested: Size, applied: Size & { clamped: boolean }) {
  if (!applied.clamped) return "";
  const smaller = applied.width < requested.width - 1 || applied.height < requested.height - 1;
  return smaller ? "（已按屏幕缩小）" : "（已按最小尺寸放大）";
}
