export const bands = [60, 170, 350, 1000, 3500, 10000]         ;
                                                                                                 
export const presets = [
  { label: '平直', gains: [0, 0, 0, 0, 0, 0] },
  { label: '人声', gains: [-3, -2, 1, 3, 2, -1] },
  { label: '音乐', gains: [3, 2, -1, 0, 2, 3] },
  { label: '影院', gains: [4, 2, -1, 1, 2, 2] },
]         ;
export const neutral = ()           => ({ gains: bands.map(() => 0), delayMs: 0, normalize: false, clarity: 0 });
export const clone = (value          )           => ({ ...value, gains: [...value.gains] });
export function clamp(value         , min        , max        ) {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new TypeError('Expected finite numeric value');
  return Math.min(max, Math.max(min, value));
}
export function nodes(value          ) {
  return [
    ...bands.flatMap((frequency, i) => value.gains[i] === 0 ? [] : [{ kind: 'equalizer'         , frequency, q: 1         , gainDb: clamp(value.gains[i], -6, 6) }]),
    ...(value.clarity === 0 ? [] : [{ kind: 'treble'         , frequency: 3000         , slope: 0.5         , gainDb: clamp(value.clarity, 0, 6) }]),
    ...(value.normalize ? [{ kind: 'dynaudnorm'         , enabled: true, maxGain: 4, peak: 0.9 }] : []),
  ];
}
