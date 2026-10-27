import type { RenderSettings, VideoNode } from "@vestra-player/sdk";

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
export interface Effects {
  sharpen: Toggle;
  edge: Toggle;
  denoise: Record<DenoiseMode, Toggle> & { shown: DenoiseMode };
  deblock: Toggle;
  deband: Toggle;
  /** E18 倍帧: the switch only; the node also needs the source frame rate (`blendFps`). */
  frameBlend: boolean;
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
  frameBlend: false,
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
  { id: "auto", label: "Auto", osd: "Deinterlace: auto" },
  { id: "on", label: "Always", osd: "Deinterlace: always" },
  { id: "off", label: "Never", osd: "Deinterlace: never" },
] as const;
export type DeinterlaceMode = (typeof DEINTERLACE_MODES)[number]["id"];
export const DEINTERLACE_OUTPUTS = [
  { id: "frame", label: "By Frame", osd: "Deinterlace output: by frame" },
  { id: "field", label: "By Field (Double Rate)", osd: "Deinterlace output: by field (double rate)" },
] as const;
export type DeinterlaceOutput = (typeof DEINTERLACE_OUTPUTS)[number]["id"];
export type Deinterlace = { mode: DeinterlaceMode; output: DeinterlaceOutput };

/** `label` is the ratio itself (not translated); `pad` and `crop` are the 裁剪/拉伸 menu titles. */
export const RATIOS = [
  { label: "4:3", aspect: 4 / 3, pad: "Stretch to 4:3", crop: "Crop to 4:3" },
  { label: "16:9", aspect: 16 / 9, pad: "Stretch to 16:9", crop: "Crop to 16:9" },
  { label: "16:10", aspect: 16 / 10, pad: "Stretch to 16:10", crop: "Crop to 16:10" },
  { label: "1.85:1", aspect: 1.85, pad: "Stretch to 1.85:1", crop: "Crop to 1.85:1" },
  { label: "2.35:1", aspect: 2.35, pad: "Stretch to 2.35:1", crop: "Crop to 2.35:1" },
] as const;
export const REFRAME_MODES = ["none", "pad", "crop"] as const;
export type ReframeMode = (typeof REFRAME_MODES)[number];
/** E12: pad or crop to one of `RATIOS`; x, y place the picture, 0–100 % (50 = centred). */
export type Reframe = { mode: ReframeMode; ratio: number; x: number; y: number };
export const POSITION_STEP = 5;

/** The OSD after a 裁剪/拉伸 change (裁剪/拉伸: 按 16:9 拉伸): a template and its argument, the ratio. */
export function reframeMessage(reframe: Reframe) {
  const args = [RATIOS[reframe.ratio]!.label];
  if (reframe.mode === "none") return { text: "Crop/stretch: none", args } as const;
  return reframe.mode === "pad" ? ({ text: "Crop/stretch: stretch to {}", args } as const) : ({ text: "Crop/stretch: crop to {}", args } as const);
}

// ---------------------------------------------------------------- 倍帧 (E18)

/** At or above this source rate 倍帧 adds no node (当前帧率已足够). */
export const BLEND_ENOUGH = 50;
/**
 * The frameBlend rate for a source rate: min(2 × rate, 60), at least the host's
 * 24; an unknown rate counts as 30. `null` when the source is smooth enough.
 */
export function blendFps(sourceFps: number | null) {
  const rate = sourceFps !== null && Number.isFinite(sourceFps) && sourceFps > 0 ? sourceFps : 30;
  if (rate >= BLEND_ENOUGH) return null;
  return Math.max(24, Math.min(Math.round(rate * 2), 60));
}

// ---------------------------------------------------------------- display layer (E11, E13)

/** `label` is the OSD text and the window's choice; the 比例 menu titles `fill` 填满窗口（裁去多余）. */
export const FITS = [
  { id: "keep", label: "Keep Aspect Ratio", menu: "Keep Aspect Ratio" },
  { id: "stretch", label: "Ignore Aspect Ratio", menu: "Ignore Aspect Ratio" },
  { id: "fill", label: "Fill Window", menu: "Fill Window (Crop Excess)" },
] as const;
export type Fit = (typeof FITS)[number]["id"];
/** E11, global: `aspect` null = 按来源; `customAspect` is the last 自定义 text ("" = none). */
export interface Display {
  aspect: number | null;
  customAspect: string;
  fit: Fit;
}
export const defaultDisplay = (): Display => ({ aspect: null, customAspect: "", fit: "keep" });
/** E13, per source: zoom 0.25–4, pan −1–1 (fractions of the shown video). */
export interface Frame {
  zoom: number;
  panX: number;
  panY: number;
}
export const centred = (): Frame => ({ zoom: 1, panX: 0, panY: 0 });
export const ZOOM_STEP = 1.05;
export const PAN_STEP = 0.01;
export const ASPECT_MIN = 0.25;
export const ASPECT_MAX = 4;

/** "21:9", "2.39" (also "21/9", full-width colon) → the ratio, or null when it is not one in 0.25–4. */
export function parseAspect(text: string): number | null {
  const t = text.trim().replace("：", ":").replace("/", ":");
  const number = /^\d+(?:\.\d+)?$|^\.\d+$/;
  let value: number;
  if (t.includes(":")) {
    const [w, h, ...rest] = t.split(":").map((part) => part.trim());
    if (rest.length > 0 || !number.test(w ?? "") || !number.test(h ?? "")) return null;
    value = Number(w) / Number(h);
  } else {
    if (!number.test(t)) return null;
    value = Number(t);
  }
  if (!Number.isFinite(value) || value < ASPECT_MIN || value > ASPECT_MAX) return null;
  return round(value);
}

/** The 宽高比 choice: 0 按来源, 1–5 `RATIOS`, 6 自定义. */
export function aspectIndex(display: Display) {
  if (display.aspect === null) return 0;
  const preset = RATIOS.findIndex((r) => Math.abs(r.aspect - display.aspect!) < 1e-3);
  return preset >= 0 ? preset + 1 : RATIOS.length + 1;
}
/** The OSD after a 宽高比 change (宽高比：默认, 宽高比：4:3 …, 宽高比：自定义 - 21:9): a template and its argument. */
export function aspectMessage(display: Display) {
  const index = aspectIndex(display);
  if (index === 0) return { text: "Aspect ratio: Default", args: [] } as const;
  return index <= RATIOS.length
    ? ({ text: "Aspect ratio: {}", args: [RATIOS[index - 1]!.label] } as const)
    : ({ text: "Aspect ratio: Custom - {}", args: [display.customAspect] } as const);
}
/** Shift+F6: 默认 → 4:3 → … → 2.35:1 → (自定义, once set) → 默认. */
export function nextAspect(display: Display): number | null {
  const custom = parseAspect(display.customAspect);
  const cycle: (number | null)[] = [null, ...RATIOS.map((r) => r.aspect), ...(custom === null ? [] : [custom])];
  const index = aspectIndex(display);
  const at = index <= RATIOS.length ? index : cycle.length - 1;
  return cycle[(at + 1) % cycle.length]!;
}

// ---------------------------------------------------------------- GPU render options (E16, E17, E18 运动补偿)

/** `osd` after a change, `sdr` the same on a source known not to be HDR. */
export const TONE_MAPPINGS = [
  { id: "auto", label: "Auto", osd: "HDR tone mapping: auto", sdr: "HDR tone mapping: auto (not an HDR source)" },
  { id: "off", label: "Off", osd: "HDR tone mapping: off", sdr: "HDR tone mapping: off (not an HDR source)" },
  { id: "bt.2390", label: "BT.2390", osd: "HDR tone mapping: BT.2390", sdr: "HDR tone mapping: BT.2390 (not an HDR source)" },
  { id: "hable", label: "Hable", osd: "HDR tone mapping: Hable", sdr: "HDR tone mapping: Hable (not an HDR source)" },
  { id: "mobius", label: "Mobius", osd: "HDR tone mapping: Mobius", sdr: "HDR tone mapping: Mobius (not an HDR source)" },
  { id: "reinhard", label: "Reinhard", osd: "HDR tone mapping: Reinhard", sdr: "HDR tone mapping: Reinhard (not an HDR source)" },
  { id: "clip", label: "Clip", osd: "HDR tone mapping: clip", sdr: "HDR tone mapping: clip (not an HDR source)" },
] as const satisfies readonly { id: RenderSettings["toneMapping"]; label: string; osd: string; sdr: string }[];
export type ToneMapping = (typeof TONE_MAPPINGS)[number]["id"];
export const PEAK_MIN = 100;
export const PEAK_MAX = 400;
export const PEAK_DEFAULT = 203;
export const MAX_SHADERS = 16;
/** Global; `shaders` are file names in `<dataDirectory>/shaders/`, kept while the switch is off. */
export interface Render {
  toneMapping: ToneMapping;
  targetPeak: number;
  peakDetect: boolean;
  shadersEnabled: boolean;
  shaders: string[];
  interpolation: boolean;
}
export const defaultRender = (): Render => ({
  toneMapping: "auto",
  targetPeak: PEAK_DEFAULT,
  peakDetect: true,
  shadersEnabled: false,
  shaders: [],
  interpolation: false,
});
/** 100–400 nits in steps of 10; 203 (BT.2408 reference white) stays as it is. */
export const peakOf = (n: number) => (n === PEAK_DEFAULT ? n : clamp(Math.round(n / 10) * 10, PEAK_MIN, PEAK_MAX));
/** `stats().video.transfer` of an HDR source (HDR10 / HLG). */
export const isHdr = (transfer: string | null | undefined) => transfer === "pq" || transfer === "hlg";
/** A user shader's file name: a plain name (no folder part, not hidden) ending in .glsl or .hook. */
export const isShaderName = (name: string) =>
  /^[^/\\.][^/\\]*\.(glsl|hook)$/i.test(name);
/** The 着色器 list after `names`: unique, at most `MAX_SHADERS`. */
export function shaderList(names: unknown): string[] {
  if (!Array.isArray(names)) return [];
  const out: string[] = [];
  for (const name of names) if (typeof name === "string" && isShaderName(name) && !out.includes(name)) out.push(name);
  return out.slice(0, MAX_SHADERS);
}

// ---------------------------------------------------------------- the chain

/** Everything the chain is computed from. */
export interface Look {
  picture: Picture;
  effects: Effects;
  deinterlace: Deinterlace;
  reframe: Reframe;
  geometry: Geometry;
  display: Display;
  render: Render;
}
export const defaultLook = (): Look => ({
  picture: { ...defaults },
  effects: defaultEffects(),
  deinterlace: { mode: "auto", output: "frame" },
  reframe: { mode: "none", ratio: 1, x: 50, y: 50 },
  geometry: upright(),
  display: defaultDisplay(),
  render: defaultRender(),
});

const round = (n: number, digits = 4) => Math.round(n * 10 ** digits) / 10 ** digits;
const strength = (t: Toggle) => (t.on ? clamp(t.strength, 0, 100) : 0);
/** The eq fields (brightness, contrast, saturation, gamma; hue has its own node). */
const EQ_FIELDS = ["brightness", "contrast", "saturation", "gamma"] as const;
/** Every eq field at its default: the node would change nothing. */
export const eqNeutral = (p: Picture) => EQ_FIELDS.every((field) => clamp(p[field], -100, 100) === 0);

/**
 * One fixed order, never user-reorderable: deinterlace → reframe → transform → denoise
 * (3D, 渐进, 瞬态) → deblock → deband → eq → hue → sharpen → edge sharpen → gblur.
 * Neutral or switched-off effects make no node: with every colour at its default there is
 * no eq node either (any node in the chain makes the player decode with copy-back, which
 * costs zero-copy hardware decoding and slows exact seeks). `keepEq` keeps an eq node at its
 * neutral values anyway: a slider dragged through 0 then updates in place instead of
 * removing and re-adding the node (each a chain rebuild) in the middle of the drag.
 * `blend` (E18, from `blendFps`) adds frameBlend last; the host keeps it last anyway.
 */
export function nodesFor(look: Look, blend: number | null = null, keepEq = false): VideoNode[] {
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
    // The host rotates first and flips after; the user flips first, so a
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
  if (keepEq || !eqNeutral(p))
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
  if (e.frameBlend && blend !== null) nodes.push({ kind: "frameBlend", fps: blend });
  return nodes;
}

// ---------------------------------------------------------------- settings.json

/** Global settings (`settings.json`, version 1). Flip, rotation and zoom/pan are per source and not saved. */
export interface Stored {
  picture: Picture;
  effects: Effects;
  deinterlace: Deinterlace;
  reframe: Reframe;
  display: Display;
  render: Render;
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
    frameBlend: e.frameBlend === true,
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
  const v = record(value.view);
  const customAspect = typeof v.customAspect === "string" && parseAspect(v.customAspect) !== null ? v.customAspect.trim() : "";
  const aspect =
    typeof v.aspect === "number" && Number.isFinite(v.aspect) && v.aspect >= ASPECT_MIN && v.aspect <= ASPECT_MAX ? round(v.aspect) : null;
  const display: Display = {
    aspect,
    // A ratio that is neither a preset nor the custom text becomes the custom text.
    customAspect:
      aspect !== null && aspectIndex({ aspect, customAspect, fit: "keep" }) > RATIOS.length && parseAspect(customAspect) !== aspect
        ? String(aspect)
        : customAspect,
    fit: oneOf(v.fit, FITS.map((f) => f.id), "keep"),
  };
  const rr = record(value.render);
  const render: Render = {
    toneMapping: oneOf(rr.toneMapping, TONE_MAPPINGS.map((t) => t.id), "auto"),
    targetPeak: peakOf(num(rr.targetPeak, PEAK_DEFAULT, PEAK_MIN, PEAK_MAX)),
    peakDetect: rr.peakDetect !== false,
    shadersEnabled: rr.shadersEnabled === true,
    shaders: shaderList(rr.shaders),
    interpolation: rr.interpolation === true,
  };
  // 倍帧 and 运动补偿 exclude each other; a file with both keeps 倍帧.
  if (effects.frameBlend) render.interpolation = false;
  const tab = typeof value.tab === "number" && Number.isInteger(value.tab) && value.tab >= 0 && value.tab < TABS ? value.tab : 0;
  return { picture, effects, deinterlace, reframe, display, render, tab };
}

const stored = (look: Look): Omit<Stored, "tab"> => ({
  picture: look.picture,
  effects: look.effects,
  deinterlace: look.deinterlace,
  reframe: look.reframe,
  display: look.display,
  render: look.render,
});

/** The display settings are written as `view` (`view.aspect`, `view.customAspect`, `view.fit`). */
export const serialize = (look: Look, tab: number) => {
  const { display, ...rest } = structuredClone(stored(look));
  return { version: 1, ...rest, view: display, tab };
};

// ---------------------------------------------------------------- frame drops (E10 field output)

export type FrameCount = { shown: number; dropped: number };
/** More than 5 % of the frames since the last count were dropped. */
export function droppedTooMany(before: FrameCount, after: FrameCount) {
  const dropped = after.dropped - before.dropped;
  const total = dropped + (after.shown - before.shown);
  return total > 0 && dropped > 0 && dropped / total > 0.05;
}
