import type { AudioNode } from "../../sdk/plugin/index.ts";

export const bands = [60, 170, 350, 1000, 3500, 10000] as const;
/** `equalizer: false` bypasses the bands but keeps their gains (PotPlayer 均衡器 Shift+E). */
export type Settings = { equalizer: boolean; gains: number[]; delayMs: number; normalize: boolean; clarity: number };
export const presets = [
  { label: "平直", gains: [0, 0, 0, 0, 0, 0] },
  { label: "人声", gains: [-3, -2, 1, 3, 2, -1] },
  { label: "音乐", gains: [3, 2, -1, 0, 2, 3] },
  { label: "影院", gains: [4, 2, -1, 1, 2, 2] },
] as const;
export const neutral = (): Settings => ({ equalizer: true, gains: bands.map(() => 0), delayMs: 0, normalize: false, clarity: 0 });

/** Rejects non-finite input (the edit is ignored), otherwise clamps. */
export function clamp(value: unknown, min: number, max: number) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("Expected finite numeric value");
  return Math.min(max, Math.max(min, value));
}

/** True when the equalizer gains equal the preset's (menu check mark). */
export const matches = (settings: Settings, index: number) =>
  presets[index]!.gains.every((gain, i) => gain === settings.gains[i]);

/** Index of the preset the gains equal, or -1 for custom gains. */
export const presetOf = (settings: Settings) => presets.findIndex((_, index) => matches(settings, index));

/** Neutral or bypassed bands and effects are left out, so neutral settings are an empty chain. */
export function nodes(value: Settings): AudioNode[] {
  return [
    ...bands.flatMap((frequency, i): AudioNode[] =>
      !value.equalizer || value.gains[i] === 0 ? [] : [{ kind: "equalizer", frequency, q: 1, gainDb: clamp(value.gains[i], -6, 6) }],
    ),
    ...(value.clarity === 0
      ? []
      : [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: clamp(value.clarity, 0, 6) } as const]),
    ...(value.normalize ? [{ kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 } as const] : []),
  ];
}
