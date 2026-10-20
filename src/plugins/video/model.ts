import type { VideoNode } from "../../sdk/plugin/index.ts";

// ---------------------------------------------------------------- colour (色彩 page, E02)

/** Colour sliders in −100…100 plus the Gaussian blur (sigma 0–10); presets and Q replace all of it. */
export interface Picture {
  brightness: number;
  contrast: number;
  saturation: number;
  hue: number;
  gamma: number;
  blurEnabled: boolean;
  blurStrength: number;
}

export const defaults: Picture = {
  brightness: 0,
  contrast: 0,
  saturation: 0,
  hue: 0,
  gamma: 0,
  blurEnabled: false,
  blurStrength: 3,
};

/** Presets never change the hue (E02): it is 0 in every one. */
export const presets: Record<string, Picture> = {
  original: { ...defaults },
  vivid: { ...defaults, contrast: 15, saturation: 20 },
  cinema: { ...defaults, contrast: 8, saturation: -12, gamma: -5 },
  soft: { ...defaults, contrast: -8, blurEnabled: true, blurStrength: 1.5 },
};

export const clamp = (n: number, min: number, max: number) =>
  Math.min(max, Math.max(min, Number.isFinite(n) ? n : min));

// ---------------------------------------------------------------- effects (效果 page, E04–E06)

/** A switch with a remembered 0–100 strength; off keeps the strength. */
export type Toggle = { on: boolean; strength: number };
export const DENOISE_MODES = ["d3", "gradual", "temporal"] as const;
export type DenoiseMode = (typeof DENOISE_MODES)[number];
export const denoiseLabels: Record<DenoiseMode, string> = { d3: "3D 降噪", gradual: "渐进降噪", temporal: "瞬态降噪" };
export interface Effects {
  sharpen: Toggle;
  edge: Toggle;
  denoise: Record<DenoiseMode, Toggle> & { shown: DenoiseMode };
  deblock: Toggle;
  deband: Toggle;
}
export const defaultEffects = (): Effects => ({
  sharpen: { on: false, strength: 50 },
  edge: { on: false, strength: 50 },
  denoise: {
    d3: { on: false, strength: 40 },
    gradual: { on: false, strength: 40 },
    temporal: { on: false, strength: 40 },
    shown: "d3",
  },
  deblock: { on: false, strength: 25 },
  deband: { on: false, strength: 40 },
});

/**
 * hqdn3d weights per strength point (luma/chroma spatial, luma/chroma temporal).
 * 3D keeps hqdn3d's own 4:3:6:4.5 ratio (40 = its defaults); 渐进 is a milder
 * balanced mix; 瞬态 is mostly temporal with a little spatial smoothing.
 */
const DENOISE_WEIGHTS: Record<DenoiseMode, readonly [number, number, number, number]> = {
  d3: [0.1, 0.075, 0.15, 0.1125],
  gradual: [0.06, 0.045, 0.06, 0.045],
  temporal: [0.02, 0.015, 0.2, 0.15],
};

// ---------------------------------------------------------------- picture (画面 page, E08–E10, E12)

export const ROTATIONS = [0, 90, 180, 270] as const;
export type Rotation = (typeof ROTATIONS)[number];
/** Flip and rotation are per source: a new source starts upright and unflipped. */
export interface Geometry {
  flipH: boolean;
  flipV: boolean;
  rotate: Rotation;
}
export const upright = (): Geometry => ({ flipH: false, flipV: false, rotate: 0 });

export const DEINTERLACE_MODES = [
  { id: "auto", label: "自动" },
  { id: "on", label: "总是使用" },
  { id: "off", label: "不使用" },
] as const;
export type DeinterlaceMode = (typeof DEINTERLACE_MODES)[number]["id"];
export const DEINTERLACE_OUTPUTS = [
  { id: "frame", label: "按帧" },
  { id: "field", label: "按场（倍帧率）" },
] as const;
export type DeinterlaceOutput = (typeof DEINTERLACE_OUTPUTS)[number]["id"];
export type Deinterlace = { mode: DeinterlaceMode; output: DeinterlaceOutput };

export const RATIOS = [
  { label: "4:3", aspect: 4 / 3 },
  { label: "16:9", aspect: 16 / 9 },
  { label: "16:10", aspect: 16 / 10 },
  { label: "1.85:1", aspect: 1.85 },
  { label: "2.35:1", aspect: 2.35 },
] as const;
export const REFRAME_MODES = ["none", "pad", "crop"] as const;
export type ReframeMode = (typeof REFRAME_MODES)[number];
/** E12: pad or crop to one of `RATIOS`; x, y place the picture, 0–100 % (50 = centred). */
export type Reframe = { mode: ReframeMode; ratio: number; x: number; y: number };
export const POSITION_STEP = 5;

export function reframeLabel(reframe: Reframe) {
  if (reframe.mode === "none") return "不使用";
  return `按 ${RATIOS[reframe.ratio]!.label} ${reframe.mode === "pad" ? "拉伸" : "裁剪"}`;
}

// ---------------------------------------------------------------- the chain

/** Everything the chain is computed from. */
export interface Look {
  picture: Picture;
  effects: Effects;
  deinterlace: Deinterlace;
  reframe: Reframe;
  geometry: Geometry;
}
export const defaultLook = (): Look => ({
  picture: { ...defaults },
  effects: defaultEffects(),
  deinterlace: { mode: "auto", output: "frame" },
  reframe: { mode: "none", ratio: 1, x: 50, y: 50 },
  geometry: upright(),
});

const round = (n: number, digits = 4) => Math.round(n * 10 ** digits) / 10 ** digits;
const strength = (t: Toggle) => (t.on ? clamp(t.strength, 0, 100) : 0);

/**
 * One fixed order, never user-reorderable: deinterlace → reframe → transform → denoise
 * (3D, 渐进, 瞬态) → deblock → deband → eq → hue → sharpen → edge sharpen → gblur.
 * Neutral or switched-off effects make no node; eq is always first among the colour nodes.
 */
export function nodesFor(look: Look): VideoNode[] {
  const { picture: p, effects: e, deinterlace, reframe, geometry: g } = look;
  const nodes: VideoNode[] = [];
  if (deinterlace.mode !== "off")
    nodes.push({ kind: "deinterlace", when: deinterlace.mode === "on" ? "all" : "interlaced", output: deinterlace.output });
  if (reframe.mode !== "none")
    nodes.push({
      kind: "reframe",
      mode: reframe.mode,
      aspect: round(RATIOS[reframe.ratio]!.aspect),
      x: clamp(reframe.x, 0, 100) / 50 - 1,
      y: clamp(reframe.y, 0, 100) / 50 - 1,
    });
  if (g.rotate !== 0 || g.flipH || g.flipV) {
    // The host rotates first and flips after; the user flips first (PotPlayer), so a
    // quarter turn swaps which axis the flip is on.
    const quarter = g.rotate === 90 || g.rotate === 270;
    nodes.push({ kind: "transform", rotate: g.rotate, flipH: quarter ? g.flipV : g.flipH, flipV: quarter ? g.flipH : g.flipV });
  }
  for (const mode of DENOISE_MODES) {
    const s = strength(e.denoise[mode]);
    if (s === 0) continue;
    const [ls, cs, lt, ct] = DENOISE_WEIGHTS[mode].map((w) => round(w * s, 3)) as [number, number, number, number];
    nodes.push({ kind: "denoise", lumaSpatial: ls, chromaSpatial: cs, lumaTemporal: lt, chromaTemporal: ct });
  }
  if (e.deblock.on) nodes.push({ kind: "deblock", strength: e.deblock.strength < 50 ? "weak" : "strong" });
  const deband = strength(e.deband);
  if (deband > 0) nodes.push({ kind: "deband", threshold: round(0.005 + (deband / 100) * 0.045), range: 16 });
  nodes.push({
    kind: "eq",
    brightness: clamp(p.brightness, -100, 100) / 100,
    contrast: 2 ** (clamp(p.contrast, -100, 100) / 100),
    saturation: 1 + clamp(p.saturation, -100, 100) / 100,
    gamma: 2 ** (clamp(p.gamma, -100, 100) / 100),
  });
  const hue = Math.round(clamp(p.hue, -100, 100));
  if (hue !== 0) nodes.push({ kind: "hue", degrees: round(hue * 1.8, 2) });
  const sharpen = strength(e.sharpen);
  if (sharpen > 0) nodes.push({ kind: "sharpen", amount: round((sharpen / 100) * 1.5), size: "small" });
  const edge = strength(e.edge);
  if (edge > 0) nodes.push({ kind: "sharpen", amount: round(edge / 100), size: "large" });
  if (p.blurEnabled && p.blurStrength > 0) nodes.push({ kind: "gblur", sigma: clamp(p.blurStrength, 0, 10), steps: 3 });
  return nodes;
}

// ---------------------------------------------------------------- settings.json

/** Global settings (`settings.json`, version 1). Flip and rotation are per source and not saved. */
export interface Stored {
  picture: Picture;
  effects: Effects;
  deinterlace: Deinterlace;
  reframe: Reframe;
  tab: number;
}
export const TABS = 4;

type Raw = Record<string, unknown>;
const record = (value: unknown): Raw =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Raw) : {};
const num = (value: unknown, fallback: number, min: number, max: number) =>
  typeof value === "number" && Number.isFinite(value) ? clamp(value, min, max) : fallback;
const toggle = (value: unknown, fallback: Toggle): Toggle => {
  const t = record(value);
  return { on: t.on === true, strength: num(t.strength, fallback.strength, 0, 100) };
};
const oneOf = <T extends string>(value: unknown, allowed: readonly T[], fallback: T): T =>
  allowed.includes(value as T) ? (value as T) : fallback;

/**
 * Lenient: a broken field falls back to its default. An unknown `version` starts
 * from the defaults (the file is only rewritten once the user changes something).
 */
export function parse(raw: unknown): Stored {
  const value = record(raw);
  const base = defaultLook();
  if (value.version !== undefined && value.version !== 1) return { ...stored(base), tab: 0 };
  const p = record(value.picture);
  const picture: Picture = {
    brightness: num(p.brightness, 0, -100, 100),
    contrast: num(p.contrast, 0, -100, 100),
    saturation: num(p.saturation, 0, -100, 100),
    hue: Math.round(num(p.hue, 0, -100, 100)),
    gamma: num(p.gamma, 0, -100, 100),
    blurEnabled: p.blurEnabled === true,
    blurStrength: num(p.blurStrength, defaults.blurStrength, 0, 10),
  };
  const e = record(value.effects);
  const d = record(e.denoise);
  const effects: Effects = {
    sharpen: toggle(e.sharpen, base.effects.sharpen),
    edge: toggle(e.edge, base.effects.edge),
    denoise: {
      d3: toggle(d.d3, base.effects.denoise.d3),
      gradual: toggle(d.gradual, base.effects.denoise.gradual),
      temporal: toggle(d.temporal, base.effects.denoise.temporal),
      shown: oneOf(d.shown, DENOISE_MODES, "d3"),
    },
    deblock: toggle(e.deblock, base.effects.deblock),
    deband: toggle(e.deband, base.effects.deband),
  };
  const di = record(value.deinterlace);
  const deinterlace: Deinterlace = {
    mode: oneOf(di.mode, DEINTERLACE_MODES.map((m) => m.id), "auto"),
    output: oneOf(di.output, DEINTERLACE_OUTPUTS.map((m) => m.id), "frame"),
  };
  const r = record(value.reframe);
  const reframe: Reframe = {
    mode: oneOf(r.mode, REFRAME_MODES, "none"),
    ratio: Math.round(num(r.ratio, 1, 0, RATIOS.length - 1)),
    x: Math.round(num(r.x, 50, 0, 100)),
    y: Math.round(num(r.y, 50, 0, 100)),
  };
  const tab = typeof value.tab === "number" && Number.isInteger(value.tab) && value.tab >= 0 && value.tab < TABS ? value.tab : 0;
  return { picture, effects, deinterlace, reframe, tab };
}

const stored = (look: Look): Omit<Stored, "tab"> => ({
  picture: look.picture,
  effects: look.effects,
  deinterlace: look.deinterlace,
  reframe: look.reframe,
});

export const serialize = (look: Look, tab: number) => ({ version: 1, ...structuredClone(stored(look)), tab });

// ---------------------------------------------------------------- frame drops (E10 field output)

export type FrameCount = { shown: number; dropped: number };
/** More than 5 % of the frames since the last count were dropped. */
export function droppedTooMany(before: FrameCount, after: FrameCount) {
  const dropped = after.dropped - before.dropped;
  const total = dropped + (after.shown - before.shown);
  return total > 0 && dropped > 0 && dropped / total > 0.05;
}
