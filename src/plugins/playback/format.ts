/** Clock text for the bar: `MM:SS`, or `H:MM:SS` from one hour; unknown is `--:--`. */
export function time(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "--:--";
  const whole = Math.max(0, Math.floor(seconds));
  const s = String(whole % 60).padStart(2, "0");
  const m = String(Math.floor(whole / 60) % 60).padStart(2, "0");
  return whole >= 3600 ? `${Math.floor(whole / 3600)}:${m}:${s}` : `${m}:${s}`;
}

/** Parse the jump editor's `HH:MM:SS.mmm` (leading parts optional); `null` when malformed. */
export function parseTime(value: string): number | null {
  if (!/^\d+(?::[0-5]\d){0,2}(?:\.\d{1,3})?$/.test(value.trim())) return null;
  const parts = value.trim().split(":").map(Number);
  const result = parts.reduce((total, part) => total * 60 + part, 0);
  return Number.isFinite(result) ? result : null;
}
