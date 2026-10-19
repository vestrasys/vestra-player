export const bands = [60, 170, 350, 1000, 3500, 10000] as const;
export type Settings = { gains: number[]; delayMs: number; normalize: boolean; clarity: number };
export const presets = [
  { label: '平直', gains: [0, 0, 0, 0, 0, 0] },
  { label: '人声', gains: [-3, -2, 1, 3, 2, -1] },
  { label: '音乐', gains: [3, 2, -1, 0, 2, 3] },
  { label: '影院', gains: [4, 2, -1, 1, 2, 2] },
] as const;
export const neutral = (): Settings => ({ gains: bands.map(() => 0), delayMs: 0, normalize: false, clarity: 0 });
export const clone = (value: Settings): Settings => ({ ...value, gains: [...value.gains] });
export function clamp(value: unknown, min: number, max: number) {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new TypeError('Expected finite numeric value');
  return Math.min(max, Math.max(min, value));
}
export function nodes(value: Settings) {
  return [
    ...bands.flatMap((frequency, i) => value.gains[i] === 0 ? [] : [{ kind: 'equalizer' as const, frequency, q: 1 as const, gainDb: clamp(value.gains[i], -6, 6) }]),
    ...(value.clarity === 0 ? [] : [{ kind: 'treble' as const, frequency: 3000 as const, slope: 0.5 as const, gainDb: clamp(value.clarity, 0, 6) }]),
    ...(value.normalize ? [{ kind: 'dynaudnorm' as const, enabled: true, maxGain: 4, peak: 0.9 }] : []),
  ];
}
