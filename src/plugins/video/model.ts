import type { VideoNode } from "../../sdk/plugin/index.ts";

export interface Picture {
  brightness: number;
  contrast: number;
  saturation: number;
  gamma: number;
  blurEnabled: boolean;
  blurStrength: number;
}

export const defaults: Picture = {
  brightness: 0,
  contrast: 0,
  saturation: 0,
  gamma: 0,
  blurEnabled: false,
  blurStrength: 3,
};

export const presets: Record<string, Picture> = {
  original: { ...defaults },
  vivid: { ...defaults, contrast: 15, saturation: 20 },
  cinema: { ...defaults, contrast: 8, saturation: -12, gamma: -5 },
  soft: { ...defaults, contrast: -8, blurEnabled: true, blurStrength: 1.5 },
};

export const clamp = (n: number, min: number, max: number) =>
  Math.min(max, Math.max(min, Number.isFinite(n) ? n : min));

/** Slider units (-100..100) to one ordered eq chain, plus optional blur. */
export function nodesFor(p: Picture): VideoNode[] {
  const nodes: VideoNode[] = [
    {
      kind: "eq",
      brightness: clamp(p.brightness, -100, 100) / 100,
      contrast: 2 ** (clamp(p.contrast, -100, 100) / 100),
      saturation: 1 + clamp(p.saturation, -100, 100) / 100,
      gamma: 2 ** (clamp(p.gamma, -100, 100) / 100),
    },
  ];
  if (p.blurEnabled && p.blurStrength > 0)
    nodes.push({ kind: "gblur", sigma: clamp(p.blurStrength, 0, 10), steps: 3 });
  return nodes;
}
