import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import type {
  Disposable,
  FilterStatus,
  PanelState,
  Plugin,
  PluginError,
  RenderingPreference,
  RenderingState,
  Scope,
  Text,
  UpscalingState,
} from "@vestra-player/sdk";
import {
  DEINTERLACE_MODES,
  FITS,
  MAX_SHADERS,
  PAN_STEP,
  PEAK_DEFAULT,
  TONE_MAPPINGS,
  ZOOM_STEP,
  DEINTERLACE_OUTPUTS,
  DENOISE_MODES,
  POSITION_STEP,
  RATIOS,
  REFRAME_MODES,
  ROTATIONS,
  activeDenoise,
  aspectIndex,
  aspectMessage,
  blendFps,
  centred,
  clamp,
  defaultEffects,
  defaultLook,
  defaults,
  droppedTooMany,
  framed,
  framesIndex,
  isHdr,
  isShaderName,
  nextAspect,
  nodesFor,
  parse,
  parseAspect,
  peakOf,
  presets,
  reframeMessage,
  rememberDenoise,
  serialize,
  switchDenoise,
  upright,
} from "./model.ts";
import type { DeinterlaceMode, DeinterlaceOutput, DenoiseMode, Fit, Frame, FrameCount, Look, ReframeMode, Rotation, Toggle, ToneMapping } from "./model.ts";

type Color = "brightness" | "contrast" | "saturation" | "hue" | "gamma";
const colors: readonly Color[] = ["brightness", "contrast", "saturation", "hue", "gamma"];
/**
 * Keyboard steps (W/E, R/T, Y/U; 色调 I/O): field, keys for −1 and +1, menu order, the menu
 * titles and the OSD templates (the new value as `{}`; `…Off` while 启用画面效果 is off).
 */
const steps = [
  {
    field: "brightness",
    keys: ["W", "E"],
    order: 442,
    down: { title: "Brightness -1%", osd: "Brightness -1% ({}%)", off: "Brightness -1% ({}%) (picture effects off)" },
    up: { title: "Brightness +1%", osd: "Brightness +1% ({}%)", off: "Brightness +1% ({}%) (picture effects off)" },
  },
  {
    field: "contrast",
    keys: ["R", "T"],
    order: 444,
    down: { title: "Contrast -1%", osd: "Contrast -1% ({}%)", off: "Contrast -1% ({}%) (picture effects off)" },
    up: { title: "Contrast +1%", osd: "Contrast +1% ({}%)", off: "Contrast +1% ({}%) (picture effects off)" },
  },
  {
    field: "saturation",
    keys: ["Y", "U"],
    order: 446,
    down: { title: "Saturation -1%", osd: "Saturation -1% ({}%)", off: "Saturation -1% ({}%) (picture effects off)" },
    up: { title: "Saturation +1%", osd: "Saturation +1% ({}%)", off: "Saturation +1% ({}%) (picture effects off)" },
  },
  {
    field: "hue",
    keys: ["I", "O"],
    order: 448,
    down: { title: "Hue -1%", osd: "Hue -1% ({}%)", off: "Hue -1% ({}%) (picture effects off)" },
    up: { title: "Hue +1%", osd: "Hue +1% ({}%)", off: "Hue +1% ({}%) (picture effects off)" },
  },
] as const satisfies readonly { field: Color; keys: readonly [string, string]; order: number; down: StepTexts; up: StepTexts }[];
type StepTexts = { title: string; osd: string; off: string };
type Step = (typeof steps)[number]["up" | "down"];
/** Effect rows with one 0–100 strength; their filters rebuild, so a drag commits on release. */
type Strength = "sharpen" | "edge" | DenoiseMode | "deblock" | "deband";
const strengths: readonly Strength[] = ["sharpen", "edge", ...DENOISE_MODES, "deblock", "deband"];
/** 图像处理 switches with a strength: menu title, key, OSD on and off; `toggle` names the setting. */
const EFFECTS = [
  { id: "video.sharpen", toggle: "sharpen", title: "Sharpen", group: "video-sharpness", bindings: ["Ctrl+R"], on: "Sharpen: on", off: "Sharpen: off" },
  { id: "video.edge", toggle: "edge", title: "Edge Sharpen", group: "video-sharpness", bindings: [], on: "Edge sharpen: on", off: "Edge sharpen: off" },
  { id: "video.deblock", toggle: "deblock", title: "Deblock", group: "video-cleanup", bindings: ["Ctrl+H"], on: "Deblock: on", off: "Deblock: off" },
  { id: "video.deband", toggle: "deband", title: "Deband", group: "video-cleanup", bindings: [], on: "Deband: on", off: "Deband: off" },
  { id: "video.denoise.d3", toggle: "d3", title: "3D Denoise", group: "video-cleanup", bindings: ["Ctrl+N"], on: "3D denoise: on", off: "3D denoise: off" },
  {
    id: "video.denoise.gradual",
    toggle: "gradual",
    title: "Gradual Denoise",
    group: "video-cleanup",
    bindings: [],
    on: "Gradual denoise: on",
    off: "Gradual denoise: off",
  },
  {
    id: "video.denoise.temporal",
    toggle: "temporal",
    title: "Temporal Denoise",
    group: "video-cleanup",
    bindings: ["Ctrl+M"],
    on: "Temporal denoise: on",
    off: "Temporal denoise: off",
  },
] as const;
type Switch = (typeof EFFECTS)[number];
/** 图像旋转 menu titles. */
const ROTATION_TITLES = [
  { degrees: 0, title: "No Rotation" },
  { degrees: 90, title: "90°" },
  { degrees: 180, title: "180°" },
  { degrees: 270, title: "270°" },
] as const satisfies readonly { degrees: Rotation; title: string }[];
/** E16, E17 and 运动补偿 are GPU render options; software rendering (and Windows for now) refuses them. */
const SOFTWARE = "Needs GPU rendering (currently software rendering)";
/** Deinterlace Auto is the player's deinterlacer, always at field rate: Output is greyed (the command's reason; the panel has the same tip). */
const AUTO_OUTPUT = "Auto outputs by field (double rate); this setting applies to Always";
/** The renderer switch (render modes §8.4): its title, and the title while a restart would change the renderer. */
const GPU_TITLE = "GPU Rendering";
const GPU_PENDING_TITLE = "GPU Rendering (Restart to Apply)";
/** Busy reason of the switch while a choice is being saved or the restart question is open. */
const SWITCHING = "Switching…";
/** Checked when the saved choice is the GPU, or Auto where the next start draws on the GPU (macOS). */
const gpuChosen = (state: RenderingState) => state.preference === "gpu" || (state.preference === "auto" && state.nextMode === "gpu");
/** E18: the source rate is read this long after the chain without frameBlend is in place, this many times at most. */
const MEASURE_DELAY_MS = 400;
const MEASURE_TRIES = 8;
/** 帧位 偏移 OSD per direction (帧位调整：向右位移 (1%, 0%)). */
const PAN = {
  right: "Framing: move right ({}%, {}%)",
  left: "Framing: move left ({}%, {}%)",
  up: "Framing: move up ({}%, {}%)",
  down: "Framing: move down ({}%, {}%)",
} as const;
const percent = (n: number) => Math.round(n * 100);
/** The value of a colour step as shown: +5, 0, -3 (data in an OSD template). */
const signed = (n: number) => (n > 0 ? `+${n}` : `${n}`);
/** A timer that does not keep the plugin process alive. */
const pause = (ms: number) => new Promise<void>((done) => void setTimeout(done, ms).unref?.());

/**
 * Picture tool: the Picture panel in the adjust column (Alt+1), one owned video filter, the
 * display layer (aspect, fit, zoom, pan) and the render options. Colour, effects, deinterlace,
 * crop/pad, aspect and fit are global (`settings.json`); flip, rotation, zoom and pan belong to
 * the current source and reset when it changes. Every command is in the command palette; only
 * Picture… is in the menu.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const store = settingsStore(plugin);
  const { advanced: savedAdvanced, ...saved } = parse(store.value);
  const look: Look = { ...defaultLook(), ...saved };
  /** The panel's Advanced section is open (saved). */
  let advanced = savedAdvanced;
  let enabled = true;
  /** E13 zoom and pan: per source, never saved. */
  let frame: Frame = centred();

  const filter = plugin.video.filter("picture");
  const view = plugin.video.view();
  // The adjust column's Picture tab (player 0.7.6); the tab's key is Picture…'s (Alt+1).
  const panel = plugin.panels.mount("adjust", "picture.slint", { title: "Picture" });
  const aspectWindow = plugin.windows.create("ui/aspect.slint");
  const shadersWindow = plugin.windows.create("ui/shaders.slint");
  const persist = () => store.save(serialize(look, advanced));

  // E16, E17, E18 运动补偿: GPU render options, global and saved; not part of the chain, so
  // 启用画面效果 leaves them alone. Under software rendering every set is refused (`unsupported`),
  // the handle's first take included. That is a state of the player, not an error: the handle
  // lives in its own scope, which keeps `unsupported` out of the log (the items show the reason
  // instead) and logs every other failure as before. The saved options stay for GPU rendering.
  const renderScope = plugin.scope();
  renderScope.onError((error) => {
    if (error.code !== "unsupported") console.error(`[${plugin.id}]`, error);
  });
  const render = renderScope.video.render();
  const shaderFolder = join(plugin.dataDirectory, "shaders");
  const shaderPath = (name: string) => join(shaderFolder, name);
  const gpuReady = () => !(render.status.state === "failed" && render.status.error.code === "unsupported");
  const gpuReason = () => (gpuReady() ? null : SOFTWARE);
  let renderSent = "";
  /** Set by a user action, so a conflict at activation stays off screen (as with the display layer). */
  let renderAnnounce = false;
  /** Missing shader files are left out (the player refuses them); 重新载入着色器 drops them from the list. */
  const applyRender = (user = false, force = false) => {
    // Software rendering: nothing is sent (it would only be refused); the settings stay saved.
    if (!gpuReady()) return;
    renderAnnounce ||= user;
    const values = {
      toneMapping: look.render.toneMapping,
      targetPeak: look.render.targetPeak,
      peakDetect: look.render.peakDetect,
      shaders: look.render.shadersEnabled ? look.render.shaders.map(shaderPath).filter((path) => existsSync(path)) : [],
      interpolation: look.render.interpolation,
    };
    const key = JSON.stringify(values);
    if (!force && key === renderSent && render.status.state !== "failed") return;
    renderSent = key;
    render.set(values);
  };
  const renderChanged = (user = false, force = false) => {
    persist();
    applyRender(user, force);
  };
  render.onStatus((status) => {
    if (status.state !== "failed") {
      if (status.state !== "applying") renderAnnounce = false;
      return;
    }
    const announce = renderAnnounce;
    renderAnnounce = false;
    // A compile error arrives after `applied`, when the panel is usually closed: always said.
    if (shaderFailed(status.error)) shown(plugin.osd.show("Shader failed to load"));
    else if (announce && status.error.code === "conflict") shown(plugin.osd.show("Render options unavailable: another plugin controls them"));
    else if (announce && status.error.code !== "unsupported")
      // The player's message, as before (e.g. 渲染选项不可用：<reason>).
      shown(plugin.osd.show("Render options unavailable: {}", [status.error.message]));
  });

  // E16: the source's transfer (`stats().video.transfer`), read once per source when it is ready.
  let transfer: string | null = null;
  let transferSource = -1;
  let transferRead = false;
  /** The source's width / height once read (null: unknown, the editor's still keeps its own 16:9). */
  let sourceAspect: number | null = null;
  let reading = false;
  let stopped = false;
  plugin.onDispose(() => {
    stopped = true;
  });
  const readTransfer = async () => {
    const source = player.state.sourceId;
    if (source !== transferSource) {
      transferSource = source;
      transfer = null;
      transferRead = false;
      sourceAspect = null;
    }
    if (transferRead || reading || player.state.phase !== "ready") return;
    reading = true;
    try {
      for (let attempt = 0; attempt < MEASURE_TRIES && !stopped && player.state.sourceId === source; attempt++) {
        if (attempt > 0) await pause(MEASURE_DELAY_MS);
        const stats = await player.stats().catch(() => null);
        if (stopped || player.state.sourceId !== source) return;
        // The geometry editor's picture shape: the coded size (square pixels assumed; the
        // display size would already include this plugin's crop and rotation).
        const { width, height } = stats?.video ?? {};
        if (width && height && width > 0 && height > 0 && sourceAspect !== width / height) {
          sourceAspect = width / height;
          plugin.update();
        }
        if (stats?.video?.transfer) {
          transfer = stats.video.transfer;
          plugin.update();
          break;
        }
      }
      transferRead = player.state.sourceId === source;
    } finally {
      reading = false;
    }
  };
  /** Only HDR sources (PQ, HLG) are tone mapped; said once the source is known not to be one. */
  const notHdr = () => transfer !== null && !isHdr(transfer);

  // E18: the source rate is read with no frameBlend in the chain (afterwards stats() reports the
  // blended rate), once per source and deinterlace setting (field output doubles the rate;
  // Output is part of the key under Auto too, where it does not apply: at worst one more read).
  const blendKey = () => `${player.state.sourceId}:${look.deinterlace.mode}:${look.deinterlace.output}`;
  let measured: { key: string; fps: number | null } | null = null;
  let abandoned: string | null = null;
  const blendWanted = () => enabled && look.effects.frameBlend;
  const blendRate = () => (blendWanted() && measured?.key === blendKey() ? blendFps(measured.fps) : null);
  const blendEnough = () => blendWanted() && measured?.key === blendKey() && blendFps(measured.fps) === null;

  let sent = "";
  let sentBlend: number | null = null;
  /** The chain last sent has an eq node (so a drag through 0 may keep it, see `nodesFor`). */
  let sentEq = false;
  /**
   * The SDK keeps one change in flight; an unchanged chain is not resent unless it failed.
   * `dragging`: a slider is being dragged, so an eq node already in the chain stays (neutral)
   * until the release commits; it is never added for that reason.
   */
  const apply = (dragging = false) => {
    sentBlend = blendRate();
    const nodes = enabled ? nodesFor(look, sentBlend, dragging && sentEq) : [];
    const key = JSON.stringify(nodes);
    if (key === sent && filter.status.state !== "failed") return;
    sent = key;
    sentEq = nodes.some((n) => n.kind === "eq");
    filter.set(nodes);
  };
  const change = (dragging = false) => {
    apply(dragging);
    persist();
    if (wantsMeasure()) void measure();
  };
  /** Switching 倍帧 by hand also retries a source whose picture did not show up in time. */
  const switchBlend = (on: boolean) => {
    look.effects.frameBlend = on;
    abandoned = null;
  };
  const wantsMeasure = () =>
    blendWanted() && player.state.phase === "ready" && measured?.key !== blendKey() && abandoned !== blendKey();
  let measuring = false;
  const measure = async () => {
    if (measuring) return;
    measuring = true;
    try {
      for (let attempt = 1; attempt <= MEASURE_TRIES; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, MEASURE_DELAY_MS));
        if (!wantsMeasure()) return;
        if (filter.status.state === "applying") continue;
        const key = blendKey();
        const stats = await player.stats().catch(() => null);
        if (!wantsMeasure() || key !== blendKey()) {
          attempt = 0;
          continue;
        }
        // No picture (audio only, or not decoded yet): try again; an unknown rate counts as 30.
        if (!stats?.video || (stats.video.fps === null && attempt < MEASURE_TRIES)) continue;
        measured = { key, fps: stats.video.fps };
        apply();
        plugin.update();
        return;
      }
      abandoned = blendKey();
    } finally {
      measuring = false;
    }
  };
  /** Best-effort feedback: a missing OSD (headless) must not look like a picture failure. */
  const shown = (message: Promise<void>) => void message.catch(() => undefined);
  const effects = look.effects;
  const isDenoise = (field: Strength): field is DenoiseMode => (DENOISE_MODES as readonly string[]).includes(field);
  const toggleOf = (field: Strength): Toggle => (isDenoise(field) ? effects.denoise[field] : effects[field]);
  const defaultToggle = (field: Strength): Toggle => {
    const base = defaultEffects();
    return isDenoise(field) ? base.denoise[field] : base[field];
  };
  /** The denoisers the panel's Denoise chip turns back on (see `switchDenoise`); this session only. */
  let lastDenoise: DenoiseMode[] = [];
  /**
   * One switch (a card row, Ctrl+N, Ctrl+M) applied to a toggle: when it turns the last denoiser
   * off, the chip remembers the set that was on (`rememberDenoise`), whatever the path.
   */
  const switchToggle = (toggle: Toggle, on: boolean) => {
    const before = activeDenoise(effects.denoise);
    toggle.on = on;
    lastDenoise = rememberDenoise(effects.denoise, before, lastDenoise);
  };

  // ---------------------------------------------------------------- panel
  /** The commands the panel's buttons run (`run`, by command id), filled where they are added. */
  const runs = new Map<string, () => unknown>();
  /** The first key of one of this plugin's commands, "" without one (the user's current keys). */
  const keyOf = (id: string) => plugin.commands.keys(id)[0] ?? "";
  /** The upscaler's state as last read (`plugin.video.upscaling`, below); null until then. */
  let upscaling: UpscalingState | null = null;
  /** The renderer switch as the panel shows it (set once the rendering state is read, below). */
  let gpuView = () => ({
    shown: false,
    checked: false,
    enabled: false,
    busy: false,
    pending: false,
    ongpu: false,
    nextgpu: false,
    reason: "",
    name: "",
  });
  /** The AI Upscaling switch as the panel shows it (set below, with the rendering state). */
  let upscalingView = () => ({ checked: false, enabled: false, busy: false, kind: "off", detail: "" as string | Text, tip: "" as string | Text, from: 0, to: 0, gpu: "" });
  panel.bind(() => {
    const { d3, gradual, temporal } = effects.denoise;
    return {
      ...look.picture,
      sharpenEnabled: effects.sharpen.on,
      sharpenStrength: effects.sharpen.strength,
      edgeEnabled: effects.edge.on,
      edgeStrength: effects.edge.strength,
      d3Enabled: d3.on,
      d3Strength: d3.strength,
      gradualEnabled: gradual.on,
      gradualStrength: gradual.strength,
      temporalEnabled: temporal.on,
      temporalStrength: temporal.strength,
      deblockEnabled: effects.deblock.on,
      deblockStrength: effects.deblock.strength,
      debandEnabled: effects.deband.on,
      debandStrength: effects.deband.strength,
      framesIndex: framesIndex(effects, look.render),
      blendEnough: blendEnough(),
      fitIndex: FITS.findIndex((f) => f.id === look.display.fit),
      customRatio: look.display.customAspect,
      aspectIndex: aspectIndex(look.display),
      // The geometry editor draws from these (0 = no aspect override / not known yet).
      displayAspect: look.display.aspect ?? 0,
      sourceAspect: sourceAspect ?? 0,
      reframeIndex: REFRAME_MODES.indexOf(look.reframe.mode),
      ratioIndex: look.reframe.ratio,
      ratioAspect: RATIOS[look.reframe.ratio]!.aspect,
      cropX: look.reframe.x,
      cropY: look.reframe.y,
      rotateIndex: ROTATIONS.indexOf(look.geometry.rotate),
      flipV: look.geometry.flipV,
      flipH: look.geometry.flipH,
      zoom: frame.zoom,
      panXValue: frame.panX,
      panYValue: frame.panY,
      zoomPercent: percent(frame.zoom),
      panX: percent(frame.panX),
      panY: percent(frame.panY),
      advancedOpen: advanced,
      deinterlaceIndex: DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode),
      outputIndex: DEINTERLACE_OUTPUTS.findIndex((m) => m.id === look.deinterlace.output),
      renderReady: gpuReady(),
      toneIndex: TONE_MAPPINGS.findIndex((t) => t.id === look.render.toneMapping),
      targetPeak: look.render.targetPeak,
      peakNits: look.render.targetPeak,
      peakDetect: look.render.peakDetect,
      notHdr: notHdr(),
      shadersEnabled: look.render.shadersEnabled,
      shaderCount: look.render.shaders.length,
      gpu: gpuView(),
      upscaling: upscalingView(),
      effectEnabled: enabled,
      ...statusLine(filter.status, view.status, render.status),
      // Read here, so the caps follow every key change (`commands.keys`).
      keys: {
        brightnessdown: keyOf("video.brightness.down"),
        brightnessup: keyOf("video.brightness.up"),
        contrastdown: keyOf("video.contrast.down"),
        contrastup: keyOf("video.contrast.up"),
        saturationdown: keyOf("video.saturation.down"),
        saturationup: keyOf("video.saturation.up"),
        huedown: keyOf("video.hue.down"),
        hueup: keyOf("video.hue.up"),
        sharpen: keyOf("video.sharpen"),
        edge: keyOf("video.edge"),
        blur: keyOf("video.gaussian"),
        deblock: keyOf("video.deblock"),
        deband: keyOf("video.deband"),
        // Each denoiser's own key (3D Ctrl+N, Temporal Ctrl+M); the Denoise chip has none.
        d3: keyOf("video.denoise.d3"),
        gradual: keyOf("video.denoise.gradual"),
        temporal: keyOf("video.denoise.temporal"),
        doubling: keyOf("video.frame-blend"),
        interpolation: keyOf("video.interpolation"),
        aspect: keyOf("video.aspect.cycle"),
        rotate: keyOf("video.rotate.cycle"),
        flipv: keyOf("video.flip.vertical"),
        fliph: keyOf("video.flip.horizontal"),
        zoomout: keyOf("video.frame.zoom-out"),
        zoomin: keyOf("video.frame.zoom-in"),
        panleft: keyOf("video.frame.left"),
        panup: keyOf("video.frame.up"),
        pandown: keyOf("video.frame.down"),
        panright: keyOf("video.frame.right"),
        centre: keyOf("video.frame.centre"),
        framing: keyOf("video.frame.reset"),
        deinterlace: keyOf("video.deinterlace.cycle"),
        shaders: keyOf("video.shader.use"),
        gpu: keyOf("video.gpu-rendering"),
        reset: keyOf("video.reset"),
        resetpanel: keyOf("video.resetPanel"),
      },
    };
  });
  /** Store a slider value; how it takes effect (null when the field is unknown). */
  const storeValue = (field: string, value: number) => {
    if (!Number.isFinite(value)) return null;
    if (field === "blurStrength") look.picture.blurStrength = clamp(value, 0, 10);
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = clamp(value, -100, 100);
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).strength = clamp(Math.round(value), 0, 100);
    else if (field === "targetPeak") look.render.targetPeak = peakOf(value);
    else return null;
    return field === "targetPeak" ? "render" : (strengths as string[]).includes(field) ? "rebuild" : "live";
  };
  // eq, hue and gblur change in place while playing; the other filters rebuild, so they wait for release.
  // The target brightness (a render option) follows the drag. The first move away from the defaults
  // adds the eq node (one rebuild); from then on the drag updates it in place, also through 0, and
  // the release removes it when every colour is back at its default.
  panel.on("adjust", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "render") renderChanged();
    else if (kind === "live") change(true);
  });
  panel.on("commit", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "render") renderChanged();
    else if (kind) change();
  });
  /** A slider's double-click: that slider back to its default. */
  panel.on("resetField", (field: string) => {
    if (field === "targetPeak") {
      look.render.targetPeak = PEAK_DEFAULT;
      return renderChanged();
    }
    if (field === "blurStrength") look.picture.blurStrength = defaults.blurStrength;
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = defaults[field as Color];
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).strength = defaultToggle(field as Strength).strength;
    else return;
    change();
  });
  panel.on("preset", (name: string) => {
    const preset = presets[name];
    if (!preset) return;
    look.picture = { ...preset };
    change();
  });
  panel.on("toggleEffect", (on: boolean) => {
    enabled = on === true;
    apply();
    if (wantsMeasure()) void measure();
  });
  panel.on("toggle", (field: string, on: boolean) => {
    const value = on === true;
    if (field === "blur") look.picture.blurEnabled = value;
    else if (field === "shaders") return setShaders(value, false);
    else if (field === "peakDetect") {
      look.render.peakDetect = value;
      return renderChanged(true);
    } else if (field === "flipV" || field === "flipH") look.geometry[field] = value;
    // The Denoise chip: the master switch over the three denoisers (`switchDenoise`).
    else if (field === "denoise") lastDenoise = switchDenoise(effects.denoise, value, lastDenoise);
    else if ((strengths as string[]).includes(field)) switchToggle(toggleOf(field as Strength), value);
    else return;
    change();
  });
  panel.on("choose", (field: string, index: number) => {
    if (!Number.isInteger(index) || index < 0) return;
    // Frames: Off / Doubling / Interpolate, one control over the two that exclude each other (E18).
    if (field === "frames" && index <= 2) {
      if (index === 1) return setBlend(true, false);
      if (index === 2) return gpuReady() ? setInterpolation(true, false) : undefined;
      if (effects.frameBlend) setBlend(false, false);
      if (look.render.interpolation) setInterpolation(false, false);
      return;
    }
    if (field === "rotate" && index < ROTATIONS.length) look.geometry.rotate = ROTATIONS[index]!;
    // The editor's crop ratios: Off, or a ratio, kept as crop or stretch (pad) as it was (crop
    // when there was none). `reframe` picks crop / stretch for the ratio already set.
    else if (field === "crop" && index <= RATIOS.length) {
      if (index === 0) look.reframe.mode = "none";
      else {
        if (look.reframe.mode === "none") look.reframe.mode = "crop";
        look.reframe.ratio = index - 1;
      }
    } else if (field === "reframe" && index < REFRAME_MODES.length) look.reframe.mode = REFRAME_MODES[index]!;
    else if (field === "ratio" && index < RATIOS.length) look.reframe.ratio = index;
    else if (field === "deinterlace" && index < DEINTERLACE_MODES.length) look.deinterlace.mode = DEINTERLACE_MODES[index]!.id;
    else if (field === "output" && index < DEINTERLACE_OUTPUTS.length) look.deinterlace.output = DEINTERLACE_OUTPUTS[index]!.id;
    else if (field === "aspect" && index <= RATIOS.length + (look.display.customAspect ? 1 : 0)) {
      look.display.aspect = index === 0 ? null : index <= RATIOS.length ? RATIOS[index - 1]!.aspect : parseAspect(look.display.customAspect);
      return displayChanged();
    } else if (field === "fit" && index < FITS.length) {
      look.display.fit = FITS[index]!.id;
      return displayChanged();
    } else if (field === "toneMapping" && index < TONE_MAPPINGS.length) {
      look.render.toneMapping = TONE_MAPPINGS[index]!.id;
      return renderChanged();
    } else return;
    change();
  });
  // The zoom, pan and crop buttons, Reset All and the GPU switch run their commands (OSD included).
  panel.on("run", (id: string) => void runs.get(String(id))?.());
  // The geometry editor: a drag, a corner or the wheel asks for a framing (zoom, pan) and the
  // video follows at once, without an OSD (the editor shows the numbers). A drag is one user
  // action: its first edit announces (a refusal says "Display unavailable" once), the moves
  // until `done` (the release) do not, so an occupied display does not say so on every frame.
  let editing = false;
  panel.on("frameEdit", (zoom: number, panX: number, panY: number, done: boolean) => {
    const next = framed(frame, Number(zoom), Number(panX), Number(panY));
    const changed = next.zoom !== frame.zoom || next.panX !== frame.panX || next.panY !== frame.panY;
    if (changed) {
      frame = next;
      showView(!editing);
    }
    editing = done !== true && (editing || changed);
  });
  // The editor's crop box (or, when stretching, the picture inside the bars) dragged to x, y
  // (0–100 %, 50 centred). A crop moves live while dragging; stretching (pad) rebuilds the
  // filter, so it waits for the release (`done`), as the strength sliders do.
  panel.on("cropAt", (x: number, y: number, done: boolean) => {
    if (!Number.isFinite(x) || !Number.isFinite(y)) return;
    const r = look.reframe;
    const nx = clamp(Math.round(x), 0, 100);
    const ny = clamp(Math.round(y), 0, 100);
    const moved = nx !== r.x || ny !== r.y;
    r.x = nx;
    r.y = ny;
    if (r.mode === "none") return;
    if (done === true) change();
    else if (moved && r.mode === "crop") apply();
  });
  panel.on("setAdvanced", (open: boolean) => {
    advanced = open === true;
    persist();
  });
  panel.on("customAspect", () => openCustom());
  panel.on("shaderList", () => openShaders());
  panel.on("retry", () => {
    if (filter.status.state === "failed") filter.retry();
    if (view.status.state === "failed") showView();
    if (render.status.state === "failed" && gpuReady()) applyRender(true, true);
  });

  // ---------------------------------------------------------------- display layer (E11, E13)
  /** Set only by a user action, so a refusal at activation (another owner) stays off screen. */
  let announce = false;
  const showView = (user = false) => {
    announce ||= user;
    view.set({ aspect: look.display.aspect, fit: look.display.fit, ...frame });
  };
  view.onStatus((status) => {
    if (status.state === "applied") announce = false;
    if (status.state !== "failed" || !announce) return;
    announce = false;
    if (status.error.code === "conflict") shown(plugin.osd.show("Display unavailable: another plugin controls the display"));
    // The player's message, as before (画面显示不可用：<reason>).
    else shown(plugin.osd.show("Display unavailable: {}", [status.error.message]));
  });
  const displayChanged = (user = false) => {
    persist();
    showView(user);
  };
  const setAspect = (aspect: number | null) => {
    look.display.aspect = aspect;
    displayChanged(true);
    const message = aspectMessage(look.display);
    shown(plugin.osd.show(message.text, message.args));
  };
  const setFit = (fit: Fit) => {
    look.display.fit = fit;
    displayChanged(true);
    shown(plugin.osd.show(FITS.find((f) => f.id === fit)!.label));
  };
  const zoomOf = (zoom: number) => clamp(Math.round(zoom * 10000) / 10000, 0.25, 4);
  const zoomBy = (factor: number) => {
    frame.zoom = zoomOf(frame.zoom * factor);
    showView(true);
    shown(plugin.osd.show(factor > 1 ? "Framing: larger ({}%)" : "Framing: smaller ({}%)", [percent(frame.zoom)]));
  };
  const panBy = (dx: number, dy: number, text: (typeof PAN)[keyof typeof PAN]) => {
    const pan = (n: number, d: number) => clamp(Math.round((n + d) * 100) / 100, -1, 1);
    frame.panX = pan(frame.panX, dx);
    frame.panY = pan(frame.panY, dy);
    showView(true);
    shown(plugin.osd.show(text, [percent(frame.panX), percent(frame.panY)]));
  };
  const resetFrame = (centreOnly: boolean) => {
    frame = centreOnly ? { ...frame, panX: 0, panY: 0 } : centred();
    showView(true);
    shown(plugin.osd.show(centreOnly ? "Framing: centered" : "Framing: reset"));
  };
  showView();

  // 自定义比例… (E11): a small input window; 确定 only takes a valid ratio.
  let draft = "";
  aspectWindow.bind(() => {
    const valid = parseAspect(draft) !== null;
    return { valid, invalid: draft.trim() !== "" && !valid };
  });
  aspectWindow.on("edited", (text: string) => {
    draft = String(text);
  });
  aspectWindow.on("confirm", (text: string) => {
    draft = String(text);
    const aspect = parseAspect(draft);
    if (aspect === null) return;
    look.display.customAspect = draft.trim();
    customMenu();
    setAspect(aspect);
    void aspectWindow.close();
  });
  aspectWindow.on("cancel", () => void aspectWindow.close());
  const openCustom = () => {
    draft = look.display.customAspect;
    aspectWindow.set({ value: draft });
    void aspectWindow.show();
  };

  /**
   * HDR Settings…: opens the panel with its Advanced section open (and remembered so), scrolled
   * so that the section's row is at the top (HDR is its second setting). The panel scrolls
   * whenever `advancedSerial` changes, also when it already shows Advanced.
   */
  let reveals = 0;
  const showAdvanced = async () => {
    if (!advanced) {
      advanced = true;
      persist();
    }
    reveals += 1;
    panel.set({ advancedSerial: reveals });
    plugin.update();
    await panel.show();
  };

  // ---------------------------------------------------------------- actions (menu and keys)
  /** The 图像处理 switches with a strength (Gaussian blur is part of the picture). */
  const switches = (): Toggle[] => [effects.sharpen, effects.edge, ...DENOISE_MODES.map((mode) => effects.denoise[mode]), effects.deblock, effects.deband];
  /**
   * 复位 (Q) returns the picture to normal: the colours and the blur back to their defaults, and
   * every image-processing switch off (sharpen, edges, the three denoisers, deblock, deband, 倍帧,
   * 运动补偿). Strengths stay, as when a switch is turned off by hand. Flip, rotation,
   * crop/stretch, aspect, framing, deinterlace and the pixel shaders / HDR options have their
   * own items and are left alone.
   */
  /** The colours to their defaults and every image-processing switch off; whether a switch was on. */
  const clearPicture = () => {
    const processing =
      look.picture.blurEnabled || switches().some((t) => t.on) || effects.frameBlend || look.render.interpolation;
    look.picture = { ...defaults };
    for (const toggle of switches()) toggle.on = false;
    lastDenoise = [];
    switchBlend(false);
    const interpolation = look.render.interpolation;
    look.render.interpolation = false;
    return { processing, interpolation };
  };
  const reset = () => {
    const { processing, interpolation } = clearPicture();
    change();
    if (interpolation) applyRender(true);
    shown(plugin.osd.show(processing ? "Picture reset; image processing off" : "Picture reset"));
  };
  /**
   * The panel's Reset All (coordinator ruling 2026-11-08): what Q resets, plus the Geometry section:
   * fit, aspect, crop/stretch and its position, rotation, flip, zoom and pan. The Advanced
   * preferences (deinterlace, HDR, shaders, GPU rendering) stay; so does the custom ratio typed last.
   */
  const resetPanel = () => {
    const { interpolation } = clearPicture();
    look.geometry = upright();
    look.reframe = { ...look.reframe, mode: "none", x: 50, y: 50 };
    look.display = { ...look.display, aspect: null, fit: "keep" };
    frame = centred();
    change();
    if (interpolation) applyRender(true);
    displayChanged(true);
    shown(plugin.osd.show("Picture panel reset"));
  };
  const step = (texts: Step, field: Color, delta: number) => {
    look.picture[field] = clamp(Math.round(look.picture[field]) + delta, -100, 100);
    change();
    shown(plugin.osd.show(enabled ? texts.osd : texts.off, [signed(look.picture[field])]));
  };
  const flip = (field: "flipV" | "flipH") => {
    look.geometry[field] = !look.geometry[field];
    apply();
    const on = look.geometry[field];
    if (field === "flipV") shown(plugin.osd.show(on ? "Flip vertically: on" : "Flip vertically: off"));
    else shown(plugin.osd.show(on ? "Flip horizontally: on" : "Flip horizontally: off"));
  };
  const rotate = (degrees: Rotation) => {
    look.geometry.rotate = degrees;
    apply();
    shown(degrees === 0 ? plugin.osd.show("Rotation: none") : plugin.osd.show("Rotation: {}°", [degrees]));
  };
  const switchEffect = (toggle: Toggle, texts: Switch) => {
    switchToggle(toggle, !toggle.on);
    change();
    shown(plugin.osd.show(toggle.on ? texts.on : texts.off));
  };
  const setDeinterlace = (mode: DeinterlaceMode) => {
    look.deinterlace.mode = mode;
    change();
    shown(plugin.osd.show(DEINTERLACE_MODES.find((m) => m.id === mode)!.osd));
  };
  const setOutput = (output: DeinterlaceOutput) => {
    look.deinterlace.output = output;
    change();
    shown(plugin.osd.show(DEINTERLACE_OUTPUTS.find((m) => m.id === output)!.osd));
  };
  const setReframe = (mode: ReframeMode, ratio = look.reframe.ratio) => {
    look.reframe.mode = mode;
    look.reframe.ratio = ratio;
    change();
    const message = reframeMessage(look.reframe);
    shown(plugin.osd.show(message.text, message.args));
  };
  const move = (dx: number, dy: number) => {
    const r = look.reframe;
    r.x = dx === 0 && dy === 0 ? 50 : clamp(r.x + dx, 0, 100);
    r.y = dx === 0 && dy === 0 ? 50 : clamp(r.y + dy, 0, 100);
    change();
    shown(plugin.osd.show("Crop position ({}%, {}%)", [r.x, r.y]));
  };

  // E18: 倍帧 and 运动补偿 exclude each other; switching one on switches the other off and says so.
  const setBlend = (on: boolean, menu: boolean) => {
    switchBlend(on);
    const excluded = on && look.render.interpolation;
    if (excluded) {
      look.render.interpolation = false;
      applyRender();
    }
    change();
    if (menu || excluded)
      shown(
        plugin.osd.show(
          !on
            ? "Frame doubling: off"
            : excluded
              ? "Frame doubling: on (motion interpolation turned off)"
              : enabled
                ? "Frame doubling: on"
                : "Frame doubling: on (picture effects off)",
        ),
      );
  };
  const setInterpolation = (on: boolean, menu: boolean) => {
    look.render.interpolation = on;
    const excluded = on && effects.frameBlend;
    if (excluded) {
      switchBlend(false);
      apply();
    }
    renderChanged(true);
    if (menu || excluded)
      shown(
        plugin.osd.show(!on ? "Motion interpolation: off" : excluded ? "Motion interpolation: on (frame doubling turned off)" : "Motion interpolation: on"),
      );
  };
  // E16: the options only act on HDR sources; on a known SDR source the OSD says so.
  const setTone = (id: ToneMapping) => {
    look.render.toneMapping = id;
    renderChanged(true);
    const row = TONE_MAPPINGS.find((t) => t.id === id)!;
    shown(plugin.osd.show(notHdr() ? row.sdr : row.osd));
  };
  const switchPeak = () => {
    look.render.peakDetect = !look.render.peakDetect;
    renderChanged(true);
    shown(plugin.osd.show(look.render.peakDetect ? "Dynamic peak detection: on" : "Dynamic peak detection: off"));
  };
  // E17: the list is file names in `<dataDirectory>/shaders/`; the switch keeps the list.
  const setShaders = (on: boolean, menu: boolean) => {
    look.render.shadersEnabled = on;
    renderChanged(true);
    const count = look.render.shaders.length;
    if (!menu) return;
    if (!on) shown(plugin.osd.show("Pixel shaders: off"));
    else if (count === 0) shown(plugin.osd.show("Pixel shaders: no shaders"));
    else shown(plugin.osd.show("Pixel shaders: on ({})", [count]));
  };

  // A new source starts upright and unflipped, silently.
  let source = player.state.sourceId;
  player.onChange((state) => {
    if (state.sourceId === source) return;
    source = state.sourceId;
    const g = look.geometry;
    if (g.flipH || g.flipV || g.rotate !== 0) {
      look.geometry = upright();
      apply();
    }
    if (frame.zoom !== 1 || frame.panX !== 0 || frame.panY !== 0) {
      frame = centred();
      showView();
    }
  });
  // A new source (or one becoming ready) needs its own rate before 倍帧 can add the node,
  // and its transfer for the HDR note.
  player.onChange(() => {
    if (blendRate() !== sentBlend) apply();
    if (wantsMeasure()) void measure();
    void readTransfer();
  });
  void readTransfer();

  // E10 field output and E18 倍帧 raise the frame rate on the CPU: when more than 5 % of the
  // frames in 5 seconds are dropped, each says so once per source.
  const heavy = [
    // Only where Output applies (Always): under Auto "try frame output" is not a choice, and a
    // progressive file is not deinterlaced at all.
    { id: "field", active: () => enabled && look.deinterlace.mode === "on" && look.deinterlace.output === "field" },
    { id: "blend", active: () => sentBlend !== null },
  ].map((item) => ({ ...item, warned: null as number | null }));
  let counted: (FrameCount & { source: number; watching: string }) | null = null;
  const monitor = setInterval(async () => {
    const state = player.state;
    const watching = heavy.filter((h) => h.active() && h.warned !== state.sourceId);
    if (watching.length === 0 || state.phase !== "ready" || state.paused) {
      counted = null;
      return;
    }
    const stats = await player.stats().catch(() => null);
    if (!stats || stats.video === null || stats.framesDropped === null) return;
    const now = { source: state.sourceId, shown: stats.framesShown, dropped: stats.framesDropped, watching: watching.map((h) => h.id).join() };
    if (counted && counted.source === now.source && counted.watching === now.watching && droppedTooMany(counted, now)) {
      for (const h of watching) h.warned = now.source;
      const said = (id: string) => watching.some((h) => h.id === id);
      shown(
        plugin.osd.show(
          said("field") && said("blend")
            ? "Many frames dropped with field output, try frame output; many dropped with frame doubling, consider turning it off"
            : said("field")
              ? "Many frames dropped with field output, try frame output"
              : "Many frames dropped with frame doubling, consider turning it off",
        ),
      );
    }
    counted = now;
  }, 5000);
  monitor.unref?.();
  plugin.onDispose(() => clearInterval(monitor));

  // ---------------------------------------------------------------- 着色器列表 (E17)
  /** Files in the shader folder that are not in the list (double-click adds one). */
  let available: string[] = [];
  /** The selected row of the list, −1 for none. */
  let picked = -1;
  const scanShaders = async () =>
    (await readdir(shaderFolder).catch(() => [] as string[])).filter(isShaderName).sort((a, b) => a.localeCompare(b));
  const refreshShaders = async () => {
    const files = await scanShaders();
    available = files.filter((name) => !look.render.shaders.includes(name));
    plugin.update();
    return files;
  };
  const editShaders = (edit: (list: string[]) => void, force = false) => {
    edit(look.render.shaders);
    picked = Math.min(picked, look.render.shaders.length - 1);
    renderChanged(true, force);
    void refreshShaders();
  };
  const addShaderName = (name: string) => {
    if (!look.render.shaders.includes(name) && look.render.shaders.length >= MAX_SHADERS) {
      shown(plugin.osd.show("At most {} shaders", [MAX_SHADERS]));
      return false;
    }
    editShaders((list) => {
      if (!list.includes(name)) list.push(name);
      picked = list.indexOf(name);
    }, true);
    return true;
  };
  /** 添加…: any .glsl file, copied into the shader folder so moving the original does not break the list. */
  const addShader = async () => {
    const path = await plugin.dialogs.openFile({ title: "Add Shader (.glsl)" }).catch(() => null);
    if (path === null) return;
    const name = basename(path);
    if (!isShaderName(name)) return shown(plugin.osd.show("Only .glsl or .hook shader files can be added"));
    if (!look.render.shaders.includes(name) && look.render.shaders.length >= MAX_SHADERS)
      return shown(plugin.osd.show("At most {} shaders", [MAX_SHADERS]));
    try {
      await mkdir(shaderFolder, { recursive: true });
      if (resolve(path) !== resolve(shaderPath(name))) await copyFile(path, shaderPath(name));
    } catch (error) {
      return shown(plugin.osd.show("Could not copy the shader: {}", [error instanceof Error ? error.message : String(error)]));
    }
    addShaderName(name);
  };
  const openShaderFolder = async () => {
    try {
      await mkdir(shaderFolder, { recursive: true });
      await plugin.system.openPath(shaderFolder);
    } catch (error) {
      shown(plugin.osd.show("Could not open the shader folder: {}", [error instanceof Error ? error.message : String(error)]));
    }
  };
  /** 重新载入着色器: rescan the folder, drop names whose file is gone, submit the list again. */
  const reloadShaders = async () => {
    const files = await refreshShaders();
    const missing = look.render.shaders.filter((name) => !files.includes(name));
    editShaders((list) => list.splice(0, list.length, ...list.filter((name) => files.includes(name))), true);
    // TRANSLATORS: {} is a list of file names.
    if (missing.length > 0) shown(plugin.osd.show("Removed missing shaders: {}", [plugin.i18n.list(missing)]));
    else shown(plugin.osd.show("Shaders reloaded ({})", [look.render.shaders.length]));
  };
  const openShaders = () => {
    void refreshShaders();
    void shadersWindow.show();
  };
  // Data only: the window words the heading, the missing files and the note.
  shadersWindow.bind(() => ({
    names: look.render.shaders,
    missing: look.render.shaders.map((name) => !existsSync(shaderPath(name))),
    selected: picked,
    available,
    count: look.render.shaders.length,
    max: MAX_SHADERS,
    renderReady: gpuReady(),
    shadersEnabled: look.render.shadersEnabled,
  }));
  shadersWindow.on("select", (index: number) => {
    picked = Number.isInteger(index) && index >= 0 && index < look.render.shaders.length ? index : -1;
    plugin.update();
  });
  shadersWindow.on("add", () => void addShader());
  shadersWindow.on("pick", (index: number) => {
    const name = available[index];
    if (Number.isInteger(index) && name !== undefined) addShaderName(name);
  });
  shadersWindow.on("remove", () => {
    if (picked >= 0) editShaders((list) => list.splice(picked, 1));
  });
  const moveShader = (by: number) => {
    const to = picked + by;
    if (picked < 0 || to < 0 || to >= look.render.shaders.length) return;
    editShaders((list) => {
      [list[picked], list[to]] = [list[to]!, list[picked]!];
      picked = to;
    });
  };
  shadersWindow.on("up", () => moveShader(-1));
  shadersWindow.on("down", () => moveShader(1));
  shadersWindow.on("clear", () => {
    picked = -1;
    editShaders((list) => list.splice(0, list.length));
  });
  shadersWindow.on("openFolder", () => void openShaderFolder());

  // ---------------------------------------------------------------- commands
  // Menu redesign step 4: the Video ▸ submenu and Reset ▸ Picture / Framing / Crop Position are
  // gone; their commands (same ids and keys) live in the command palette, and their controls in
  // the Picture panel, which Picture… (Alt+1, also in the context menu) opens and closes.
  /**
   * Command categories (player 0.7.6): the command palette names a command "Category: Title".
   * They are the old submenu labels (the same texts and translations), "Reset" for the old Reset
   * menu's items (the player's own word) and "Picture" for the items that were directly under Video.
   */
  const category = {
    shaders: plugin.i18n.text("Pixel Shaders"),
    hdr: plugin.i18n.text("HDR Tone Mapping"),
    deinterlace: plugin.i18n.text("Deinterlace"),
    aspect: plugin.i18n.text("Aspect"),
    crop: plugin.i18n.text("Crop/Stretch"),
    framing: plugin.i18n.text("Framing"),
    processing: plugin.i18n.text("Image Processing"),
    rotation: plugin.i18n.text("Rotation"),
    reset: plugin.i18n.text("Reset"),
    picture: plugin.i18n.text("Picture"),
  };
  /** `home`: the command that opens the panel holding this command's control (Picture…). */
  const inPicture = { home: "video.picture" } as const;
  const { commands } = plugin;
  // 402 Pixel Shaders (E17, E16): GPU render options, disabled with the reason under software
  // rendering. The list and the folder stay usable, so shaders can be arranged beforehand.
  const gpu = { enabled: gpuReady, reason: gpuReason };
  const pixel = { order: 402 } as const;
  commands.add({
    id: "video.shader.use",
    title: "Use Shaders",
    category: category.shaders,
    ...inPicture,
    ...pixel,
    group: "video-source",
    bindings: ["Shift+P"],
    checked: () => look.render.shadersEnabled,
    ...gpu,
    run: () => setShaders(!look.render.shadersEnabled, true),
  });
  const shaders = { category: category.shaders, ...pixel, group: "video-source" } as const;
  commands.add({ id: "video.shader.list", title: "Shader List…", ...shaders, ...inPicture, run: openShaders });
  commands.add({ id: "video.shader.folder", title: "Open Shader Folder", ...shaders, home: "video.shader.list", run: openShaderFolder });
  commands.add({ id: "video.shader.reload", title: "Reload Shaders", ...shaders, ...gpu, run: reloadShaders });
  for (const { id, label } of TONE_MAPPINGS)
    commands.add({
      id: `video.hdr.${id}`,
      title: label,
      category: category.hdr,
      ...inPicture,
      ...pixel,
      group: "video-hdr",
      radio: true,
      checked: () => look.render.toneMapping === id,
      ...gpu,
      run: () => setTone(id),
    });
  commands.add({
    id: "video.hdr.peak",
    title: "Dynamic Peak Detection",
    category: category.hdr,
    ...inPicture,
    ...pixel,
    group: "video-hdr-peak",
    checked: () => look.render.peakDetect,
    ...gpu,
    run: switchPeak,
  });
  // Opens the panel at its HDR rows (Advanced).
  commands.add({
    id: "video.hdr.settings",
    title: "HDR Settings…",
    category: category.shaders,
    ...inPicture,
    ...pixel,
    group: "video-hdr",
    ...gpu,
    run: showAdvanced,
  });
  // 403 Deinterlace (E10): Auto is the player's per-frame deinterlacer at field rate, Always is BWDIF
  // with the Output setting (greyed under Auto).
  commands.add({
    id: "video.deinterlace.cycle",
    title: "Cycle",
    category: category.deinterlace,
    ...inPicture,
    order: 403,
    group: "video-source",
    bindings: ["Ctrl+I"],
    run: () => {
      const index = DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode);
      setDeinterlace(DEINTERLACE_MODES[(index + 1) % DEINTERLACE_MODES.length]!.id);
    },
  });
  for (const { id, label } of DEINTERLACE_MODES)
    commands.add({
      id: `video.deinterlace.${id}`,
      title: label,
      category: category.deinterlace,
      ...inPicture,
      order: 403,
      group: "video-source",
      radio: true,
      checked: () => look.deinterlace.mode === id,
      run: () => setDeinterlace(id),
    });
  for (const { id, label } of DEINTERLACE_OUTPUTS)
    commands.add({
      id: `video.deinterlace.output.${id}`,
      title: label,
      category: category.deinterlace,
      ...inPicture,
      order: 403,
      group: "video-deinterlace-output",
      radio: true,
      enabled: () => look.deinterlace.mode !== "auto",
      reason: () => (look.deinterlace.mode === "auto" ? AUTO_OUTPUT : null),
      checked: () => look.deinterlace.output === id,
      run: () => setOutput(id),
    });
  // 404 Aspect (E11): display mode, then the aspect ratios.
  for (const { id, menu } of FITS)
    commands.add({
      id: `video.fit.${id}`,
      title: menu,
      category: category.aspect,
      ...inPicture,
      order: 404,
      group: "video-display",
      radio: true,
      checked: () => look.display.fit === id,
      run: () => setFit(id),
    });
  commands.add({
    id: "video.aspect.cycle",
    title: "Cycle Aspect Ratio",
    category: category.aspect,
    ...inPicture,
    order: 404,
    group: "video-aspect",
    bindings: ["Shift+F6"],
    run: () => setAspect(nextAspect(look.display)),
  });
  commands.add({
    id: "video.aspect.default",
    title: "Default (Source)",
    category: category.aspect,
    ...inPicture,
    order: 404,
    group: "video-aspect",
    radio: true,
    checked: () => look.display.aspect === null,
    run: () => setAspect(null),
  });
  RATIOS.forEach(({ label, aspect }, i) =>
    commands.add({
      id: `video.aspect.${i}`,
      // i18n-ignore: a ratio (4:3), the same in every language
      title: label,
      category: category.aspect,
      ...inPicture,
      order: 404,
      group: "video-aspect",
      radio: true,
      checked: () => aspectIndex(look.display) === i + 1,
      run: () => setAspect(aspect),
    }),
  );
  // The Custom item shows its ratio in the title, so it is re-added (with Custom Aspect Ratio… after it) when that changes.
  let custom = plugin.scope();
  let rebuilding: Promise<void> = Promise.resolve();
  const rebuild = async () => {
    // The ids are declared again, so the old items must be gone first.
    await custom.dispose();
    custom = plugin.scope();
    addCustom();
  };
  const customMenu = () => {
    rebuilding = rebuilding.then(rebuild, rebuild);
  };
  const addCustom = () => {
    const text = look.display.customAspect;
    const item = {
      id: "video.aspect.custom",
      category: category.aspect,
      ...inPicture,
      order: 404,
      group: "video-aspect",
      radio: true,
      checked: () => aspectIndex(look.display) === RATIOS.length + 1,
      run: () => setAspect(parseAspect(text)),
    };
    if (text) custom.commands.add({ ...item, title: plugin.i18n.text("Custom ({})", [text]) });
    else custom.commands.add({ ...item, title: "Custom (x:y)", enabled: false, reason: "Enter one with Custom Aspect Ratio… first" });
    custom.commands.add({
      id: "video.aspect.edit",
      title: "Custom Aspect Ratio…",
      category: category.aspect,
      ...inPicture,
      order: 404,
      group: "video-aspect",
      run: openCustom,
    });
  };
  addCustom();
  // 405 Crop/Stretch (E12).
  commands.add({
    id: "video.reframe.none",
    title: "None",
    category: category.crop,
    ...inPicture,
    order: 405,
    group: "video-display",
    radio: true,
    checked: () => look.reframe.mode === "none",
    run: () => setReframe("none"),
  });
  for (const mode of ["pad", "crop"] as const)
    RATIOS.forEach((row, ratio) =>
      commands.add({
        id: `video.reframe.${mode}.${ratio}`,
        title: mode === "pad" ? row.pad : row.crop,
        category: category.crop,
        ...inPicture,
        order: 405,
        group: "video-display",
        radio: true,
        checked: () => look.reframe.mode === mode && look.reframe.ratio === ratio,
        run: () => setReframe(mode, ratio),
      }),
    );
  const positions = [
    { id: "left", title: "Move Left", dx: -POSITION_STEP, dy: 0 },
    { id: "right", title: "Move Right", dx: POSITION_STEP, dy: 0 },
    { id: "up", title: "Move Up", dx: 0, dy: -POSITION_STEP },
    { id: "down", title: "Move Down", dx: 0, dy: POSITION_STEP },
  ] as const;
  for (const { id, title, dx, dy } of positions) {
    runs.set(`video.reframe.${id}`, () => move(dx, dy));
    commands.add({
      id: `video.reframe.${id}`,
      title,
      category: category.crop,
      ...inPicture,
      order: 405,
      group: "video-reframe-position",
      repeat: true,
      run: () => move(dx, dy),
    });
  }
  // The resets keep their ids, keys and "Reset: …" names; their controls are the panel's ↺ buttons.
  commands.add({
    id: "video.reframe.reset",
    title: "Crop Position",
    category: category.reset,
    ...inPicture,
    order: 442,
    group: "reset-video",
    repeat: false,
    run: () => move(0, 0),
  });
  runs.set("video.reframe.reset", () => move(0, 0));
  // 406 Framing (E13): mpv-style Alt keys first (the panel shows the first key), keypad keys too.
  const frames = [
    { id: "zoom-in", title: "Zoom In", group: "video-display", bindings: ["Alt+Plus", "Numpad9"], run: () => zoomBy(ZOOM_STEP), repeat: true },
    { id: "zoom-out", title: "Zoom Out", group: "video-display", bindings: ["Alt+-", "Numpad1"], run: () => zoomBy(1 / ZOOM_STEP), repeat: true },
    { id: "right", title: "Pan →", group: "video-frame-pan", bindings: ["Alt+ArrowRight", "Ctrl+Numpad6"], run: () => panBy(PAN_STEP, 0, PAN.right), repeat: true },
    { id: "left", title: "Pan ←", group: "video-frame-pan", bindings: ["Alt+ArrowLeft", "Ctrl+Numpad4"], run: () => panBy(-PAN_STEP, 0, PAN.left), repeat: true },
    { id: "up", title: "Pan ↑", group: "video-frame-pan", bindings: ["Alt+ArrowUp", "Ctrl+Numpad8"], run: () => panBy(0, -PAN_STEP, PAN.up), repeat: true },
    { id: "down", title: "Pan ↓", group: "video-frame-pan", bindings: ["Alt+ArrowDown", "Ctrl+Numpad2"], run: () => panBy(0, PAN_STEP, PAN.down), repeat: true },
    { id: "centre", title: "Center Frame", group: "video-frame-pan", bindings: ["Ctrl+Numpad5"], run: () => resetFrame(true), repeat: false },
  ] as const;
  for (const { id, title, group, bindings, run, repeat } of frames) {
    runs.set(`video.frame.${id}`, run);
    commands.add({
      id: `video.frame.${id}`,
      title,
      category: category.framing,
      ...inPicture,
      order: 406,
      group,
      bindings: [...bindings],
      repeat,
      run,
    });
  }
  commands.add({
    id: "video.frame.reset",
    title: "Framing",
    category: category.reset,
    ...inPicture,
    order: 441,
    group: "reset-video",
    bindings: ["Alt+Backspace", "Numpad5"],
    repeat: false,
    run: () => resetFrame(false),
  });
  runs.set("video.frame.reset", () => resetFrame(false));
  // 408 Image Processing (E04–E06, E08; Gaussian blur too, same id and key).
  commands.add({
    id: "video.flip.vertical",
    title: "Flip Vertically",
    category: category.processing,
    ...inPicture,
    order: 408,
    group: "video-effects",
    bindings: ["Ctrl+P"],
    checked: () => look.geometry.flipV,
    run: () => flip("flipV"),
  });
  commands.add({
    id: "video.flip.horizontal",
    title: "Flip Horizontally",
    category: category.processing,
    ...inPicture,
    order: 408,
    group: "video-effects",
    bindings: ["Ctrl+Z"],
    checked: () => look.geometry.flipH,
    run: () => flip("flipH"),
  });
  commands.add({
    id: "video.gaussian",
    title: "Gaussian Blur",
    category: category.processing,
    ...inPicture,
    order: 408,
    group: "video-sharpness",
    bindings: ["Ctrl+B"],
    checked: () => look.picture.blurEnabled,
    run: () => {
      look.picture.blurEnabled = !look.picture.blurEnabled;
      change();
      shown(plugin.osd.show(look.picture.blurEnabled ? "Gaussian blur: on" : "Gaussian blur: off"));
    },
  });
  const toggles: Record<(typeof EFFECTS)[number]["toggle"], () => Toggle> = {
    sharpen: () => effects.sharpen,
    edge: () => effects.edge,
    deblock: () => effects.deblock,
    deband: () => effects.deband,
    d3: () => effects.denoise.d3,
    gradual: () => effects.denoise.gradual,
    temporal: () => effects.denoise.temporal,
  };
  for (const row of EFFECTS) {
    const toggle = toggles[row.toggle];
    commands.add({
      id: row.id,
      title: row.title,
      category: category.processing,
      ...inPicture,
      order: 408,
      group: row.group,
      bindings: [...row.bindings],
      checked: () => toggle().on,
      run: () => switchEffect(toggle(), row),
    });
  }
  // E18: frame doubling adds frameBlend once the source rate is known; motion interpolation is a GPU render option.
  commands.add({
    id: "video.frame-blend",
    title: "Frame Doubling",
    category: category.processing,
    ...inPicture,
    order: 408,
    group: "video-frames",
    checked: () => effects.frameBlend,
    run: () => setBlend(!effects.frameBlend, true),
  });
  commands.add({
    id: "video.interpolation",
    title: "Motion Interpolation",
    category: category.processing,
    ...inPicture,
    order: 408,
    group: "video-frames",
    checked: () => look.render.interpolation,
    ...gpu,
    run: () => setInterpolation(!look.render.interpolation, true),
  });
  // 409 Rotation (E09).
  commands.add({
    id: "video.rotate.cycle",
    title: "Cycle",
    category: category.rotation,
    ...inPicture,
    order: 409,
    group: "video-effects",
    bindings: ["Alt+K"],
    run: () => rotate(ROTATIONS[(ROTATIONS.indexOf(look.geometry.rotate) + 1) % ROTATIONS.length]!),
  });
  for (const { degrees, title } of ROTATION_TITLES)
    commands.add({
      id: `video.rotate.${degrees}`,
      title,
      category: category.rotation,
      ...inPicture,
      order: 409,
      group: "video-effects",
      radio: true,
      checked: () => look.geometry.rotate === degrees,
      run: () => rotate(degrees),
    });
  // Picture… opens the panel (the adjust column at the Picture tab) and closes it when that tab
  // shows (Alt+1, as Alt+2…4 for the other built-in panels). Palette only (user ruling 2026-11-08):
  // the menu's Adjustments and Effects (the player's) opens the column at its last tab.
  commands.add({
    id: "video.picture",
    title: "Picture…",
    category: category.picture,
    bindings: ["Alt+1"],
    run: () => panel.toggle(),
  });
  commands.add({
    id: "video.reset",
    title: "Picture",
    category: category.reset,
    ...inPicture,
    order: 440,
    group: "reset-video",
    bindings: ["Q"],
    run: reset,
  });
  runs.set("video.reset", reset);
  // The panel's Reset All: no default key; in the palette as "Reset: Picture Panel".
  commands.add({
    id: "video.resetPanel",
    title: "Picture Panel",
    category: category.reset,
    ...inPicture,
    order: 443,
    group: "reset-video",
    run: resetPanel,
  });
  runs.set("video.resetPanel", resetPanel);
  for (const { field, keys, order, down, up } of steps) {
    // All colour steps repeat while held.
    commands.add({
      id: `video.${field}.down`,
      title: down.title,
      category: category.picture,
      ...inPicture,
      order,
      group: "video-color",
      bindings: [keys[0]],
      repeat: true,
      run: () => step(down, field, -1),
    });
    commands.add({
      id: `video.${field}.up`,
      title: up.title,
      category: category.picture,
      ...inPicture,
      order: order + 1,
      group: "video-color",
      bindings: [keys[1]],
      repeat: true,
      run: () => step(up, field, 1),
    });
  }
  apply();
  applyRender();

  // 460 GPU Rendering: the player's renderer choice (`plugin.rendering`), a switch in
  // the panel's Rendering section and a command. The player keeps the choice and applies it at the
  // next start; the switch only shows and changes it. A title cannot change in place, so the
  // command lives in a child scope rebuilt when it changes.
  let rendering: RenderingState | null = await plugin.rendering.state().catch(() => null);
  let renderingMenu: Scope | null = null;
  let renderingTitle: string | null = null;
  let switching = false;
  let restarting = false;
  /** Usable where the GPU can be chosen, and always while checked, so a user can leave the GPU. */
  const switchable = () => rendering !== null && (rendering.gpu.available || gpuChosen(rendering));
  /** "Switching…" while busy; the player's own message (data, not a text to translate) while disabled. */
  // TRANSLATORS: {} is the player's own message, already in the interface language.
  const playerText = (message: string) => plugin.i18n.text("{}", [message]);
  const switchReason = () => (switching ? SWITCHING : switchable() || !rendering?.gpu.reason ? null : playerText(rendering.gpu.reason.message));
  const buildRendering = async () => {
    const title = rendering === null ? null : rendering.restartPending ? GPU_PENDING_TITLE : GPU_TITLE;
    if (stopped || title === renderingTitle) return;
    renderingTitle = title;
    await renderingMenu?.dispose();
    renderingMenu = null;
    if (title === null || stopped) return;
    renderingMenu = plugin.scope();
    renderingMenu.commands.add({
      id: "video.gpu-rendering",
      // GPU_PENDING_TITLE or GPU_TITLE, written out so that the catalog finds them.
      title: title === GPU_PENDING_TITLE ? "GPU Rendering (Restart to Apply)" : "GPU Rendering",
      category: category.picture,
      ...inPicture,
      order: 460,
      group: "video-rendering",
      checked: () => rendering !== null && gpuChosen(rendering),
      // Greyed out with the player's reason where the GPU cannot be chosen (Windows without a
      // GPU renderer, a denylisted driver, a GPU crash at an earlier start, no user interface);
      // a checked item stays usable, so a user can always switch back to software rendering.
      enabled: switchable,
      reason: switchReason,
      busy: () => switching,
      run: switchRendering,
    });
    // Chrome's Relaunch beside its hardware acceleration switch: the restart stays offered while
    // the choice waits for it; picking the switch again undoes the choice instead.
    if (rendering?.restartPending)
      renderingMenu.commands.add({
        id: "video.gpu-rendering.restart",
        title: "Restart Player to Apply",
        category: category.picture,
        ...inPicture,
        order: 460,
        group: "video-rendering",
        busy: () => restarting || switching,
        run: restartNow,
      });
  };
  // The panel's switch: the same state as the command, with its status line (mockup 31). The
  // reason is the player's own message: why the GPU cannot be chosen, or what still rules it out
  // at the next start although it is chosen. The player's state has no GPU name; the upscaler's
  // state has one once it was read (`gpu`, the GPU that renders).
  gpuView = () => ({
    shown: rendering !== null,
    checked: rendering !== null && gpuChosen(rendering),
    enabled: switchable(),
    busy: switching || restarting,
    pending: rendering?.restartPending ?? false,
    ongpu: rendering?.mode === "gpu",
    nextgpu: rendering?.nextMode === "gpu",
    reason:
      rendering === null
        ? ""
        : !rendering.gpu.available && rendering.gpu.reason
          ? rendering.gpu.reason.message
          : gpuChosen(rendering) && rendering.nextMode !== "gpu"
            ? rendering.nextReason.message
            : "",
    name: upscaling?.gpu ?? "",
  });
  let renderingQueue: Promise<void> = Promise.resolve();
  const showRendering = () => (renderingQueue = renderingQueue.then(buildRendering, buildRendering));
  /** Read the state again (the SDK has no menu-open event): on a new source and around every switch. */
  const refreshRendering = async () => {
    const next = await plugin.rendering.state().catch(() => null);
    if (stopped) return;
    if (next) rendering = next;
    plugin.update();
    await showRendering();
  };
  const switchRendering = async () => {
    if (switching) return;
    switching = true;
    plugin.update();
    try {
      const now = await plugin.rendering.state().catch(() => rendering);
      if (now === null) return;
      rendering = now;
      const choice: RenderingPreference = gpuChosen(now) ? "software" : "gpu";
      try {
        rendering = await plugin.rendering.setPreference(choice);
      } catch (error) {
        // `unsupported`/`unavailable`: the GPU was ruled out after the state was read (the item
        // greys out with the reason); `failed`: the player could not save its settings.
        const refused = error as PluginError;
        rendering = await plugin.rendering.state().catch(() => rendering);
        if (refused.code === "unsupported" || refused.code === "unavailable")
          shown(plugin.osd.show("GPU rendering unavailable: {}", [refused.message]));
        else shown(plugin.osd.show("Could not save the rendering choice: {}", [refused.message]));
        return;
      } finally {
        plugin.update();
        await showRendering();
      }
      const on = choice === "gpu";
      // Chosen, but something still rules the GPU out at the next start (VESTRA_RENDER, a
      // denylist rule…): say why instead of "on".
      if (on && rendering.nextMode !== "gpu") shown(plugin.osd.show("GPU rendering chosen; the next start still uses software rendering: {}", [rendering.nextReason.message]));
      else if (!rendering.restartPending) shown(plugin.osd.show(on ? "GPU rendering: on" : "GPU rendering: off"));
      else await offerRestart(on);
    } finally {
      switching = false;
      plugin.update();
    }
  };
  /** The choice waits for a restart: offer it now; Later (or no dialog) keeps it for the next start. */
  const offerRestart = async (on: boolean) => {
    const answer = await plugin.dialogs
      .confirm({
        message: "Restart Vestra Player now to apply?",
        detail: on
          ? "Vestra Player will draw on the GPU from the restart on. A local file continues where it is. If you choose Later, GPU rendering starts the next time you open Vestra Player."
          : "Vestra Player will use software rendering from the restart on. A local file continues where it is. If you choose Later, software rendering starts the next time you open Vestra Player.",
        confirm: "Restart Now",
        cancel: "Later",
      })
      .catch(() => null);
    if (stopped) return;
    if (!answer?.confirmed) {
      shown(plugin.osd.show(on ? "GPU rendering: on (applies at the next start)" : "GPU rendering: off (applies at the next start)"));
      return;
    }
    await restartNow();
  };
  /** Resolves once the player accepted; it then quits (stopping this plugin) and starts again. */
  const restartNow = async () => {
    if (restarting) return;
    restarting = true;
    plugin.update();
    try {
      await plugin.rendering.restartPlayer();
    } catch (error) {
      if (!stopped) shown(plugin.osd.show("Could not restart the player: {}", [(error as PluginError).message]));
    } finally {
      restarting = false;
      if (!stopped) plugin.update();
    }
  };
  runs.set("video.gpu-rendering", switchRendering);
  runs.set("video.gpu-rendering.restart", restartNow);

  // 461 AI Upscaling: the GPU's video upscaler, NVIDIA RTX Video Super Resolution
  // (`plugin.video.upscaling`; user ruling 2026-11-08: this plugin offers it, under GPU Rendering
  // in the panel's Rendering section and as a command; there is no separate upscaling plugin).
  // The choice is saved and asked for again at each start (the player keeps a request for one run).
  // The player's first GPU check wakes a sleeping discrete GPU (hybrid laptops), so the state is
  // read only when the user wants upscaling: at start and when the panel shows while it is saved
  // on, and when the user switches (the panel switch or the command). Opening the panel alone reads
  // nothing: until then the row says Off (what it does) with the switch usable, and the GPU line
  // has no name; turned on where it cannot work, the switch goes back off with the player's reason
  // (and the command greys with it from then on). While the panel shows and the state has been
  // read, the player's changes are followed (a listener would keep this plugin running, so only
  // then). Under software rendering nothing is asked: it needs GPU rendering.
  let upscalingBusy = false;
  const softwareNow = () => rendering?.mode === "software";
  /**
   * What the status line says (`Upscaling.kind` in picture.slint) and the player's message; with
   * kind `plugins`, `detail` is the line (short, what to do: the narrowest adjust column shows
   * about 36 characters) and `tip` its tooltip (why).
   */
  const upscalingStatus = (): { kind: string; detail: string | Text; tip?: Text } => {
    const on = look.render.upscaling;
    if (rendering?.mode === "software")
      // No GPU renderer on this system at all (Linux): not on this system; else it needs GPU rendering.
      return { kind: rendering.gpu.reason?.code === "no-gpu-backend" ? "platform" : "rendering", detail: "" };
    const reason = upscaling?.reason ?? null;
    if (upscaling === null || reason?.code === "unchecked") return { kind: on ? "on" : "off", detail: "" };
    if (reason) {
      const { code, message } = reason;
      if (code === "platform") return { kind: "platform", detail: message };
      if (code === "no-gpu" || code === "driver" || code === "gpu-unsupported" || code === "adapter") return { kind: "gpu", detail: message };
      if (code === "rendering" && rendering?.mode !== "gpu") return { kind: "rendering", detail: message };
      if (!on) return { kind: "off", detail: "" };
      if (code === "decoding") {
        const { cause } = reason;
        const plugins = reason.plugins ?? [];
        // Other plugins' video filters keep the frames in memory (copy-back decoding): name them
        // (their manifest names, as the player gives them; this plugin's too when its filters are
        // also on), so the user knows whose effects to turn off. One and several are two texts:
        // the line says what to do, its tooltip why.
        if (cause === "copy-back" && plugins.some((p) => p.id !== plugin.id)) {
          const names = plugin.i18n.list(plugins.map((p) => p.name));
          return plugins.length === 1
            ? {
                kind: "plugins",
                detail: plugin.i18n.text("Waiting · turn off {}'s effects", [names]),
                tip: plugin.i18n.text("{} keeps the frames in memory for its effects, but upscaling needs them on the GPU. Turn its effects off to upscale.", [names]),
              }
            : {
                kind: "plugins",
                detail: plugin.i18n.text("Waiting · turn off the effects of {}", [names]),
                tip: plugin.i18n.text("{} keep the frames in memory for their effects, but upscaling needs them on the GPU. Turn their effects off to upscale.", [names]),
              };
        }
        // Only this plugin's own filters do: say which, so the line names what works. Deinterlace
        // Auto is the player's own deinterlacer and neutral values make no node, so with the
        // panel at its defaults nothing here waits. Deinterlace Always (bwdif) is an Advanced
        // preference that neither reset touches: "deinterlace" asks for Auto; with Colour,
        // Effects or Geometry away from their defaults too, "effects-off" asks for the master
        // switch (the one action that clears both); those alone, "effects" (Reset All).
        if (cause === "copy-back" && plugins.length > 0) {
          const always = enabled && look.deinterlace.mode === "on";
          const adjusted = enabled && nodesFor(look, blendRate()).some((n) => n.kind !== "deinterlace");
          return { kind: always ? (adjusted ? "effects-off" : "deinterlace") : "effects", detail: message };
        }
        // No plugin named: copy-back for another reason (the player has no hardware decoding setting).
        if (cause === "copy-back") return { kind: "copy-back", detail: message };
        if (cause === "hwdec-off" || cause === "software" || cause === "other-hwdec") return { kind: cause, detail: message };
        // A cause this plugin does not know: the player's message (below).
      }
      // Can pass during the run (another file, GPU rendering still starting).
      if (code === "decoding" || code === "rendering") return { kind: "waiting", detail: message };
      return { kind: "other", detail: message };
    }
    if (!on) return { kind: "off", detail: "" };
    return { kind: upscaling.requested ? "upscaling" : "idle", detail: "" };
  };
  /** It cannot work in this run: greyed (a checked switch stays usable, to turn it off). */
  const upscalingBlocked = () => ["platform", "rendering", "gpu"].includes(upscalingStatus().kind);
  const upscalingReason = () => {
    if (upscalingBusy) return SWITCHING;
    if (look.render.upscaling || !upscalingBlocked()) return null;
    const { kind, detail } = upscalingStatus();
    if (kind === "platform") return "Not available on this system (Windows with an NVIDIA RTX GPU)";
    if (kind === "rendering") return SOFTWARE;
    return plugin.i18n.text("Not available on this GPU: {}", [detail]);
  };
  upscalingView = () => {
    const { kind, detail, tip } = upscalingStatus();
    const upscaled = kind === "upscaling" && upscaling !== null;
    return {
      checked: look.render.upscaling,
      enabled: look.render.upscaling || !upscalingBlocked(),
      busy: upscalingBusy,
      kind,
      detail,
      tip: tip ?? "",
      from: upscaled ? (upscaling?.video?.displayHeight ?? 0) : 0,
      to: upscaled ? (upscaling?.area?.height ?? 0) : 0,
      gpu: upscaling?.gpu ?? "",
    };
  };
  const readUpscaling = async () => {
    if (softwareNow()) return;
    const next = await plugin.video.upscaling.state().catch(() => null);
    if (stopped) return;
    if (next) upscaling = next;
    plugin.update();
    followPanel(panel.state);
  };
  const switchUpscaling = async () => {
    if (upscalingBusy) return;
    const on = !look.render.upscaling;
    // Greyed with the reason; nothing to ask.
    if (on && upscalingBlocked()) return;
    upscalingBusy = true;
    plugin.update();
    try {
      if (on) {
        try {
          upscaling = await plugin.video.upscaling.set(true);
        } catch (error) {
          // `unsupported`: it cannot work in this run (the reason greys the switch from now on).
          const refused = error as PluginError;
          upscaling = await plugin.video.upscaling.state().catch(() => upscaling);
          // The player's reason as the state gives it (the refusal's message wraps it).
          if (!stopped) shown(plugin.osd.show("AI upscaling unavailable: {}", [upscaling?.reason?.message ?? refused.message]));
          return;
        }
        look.render.upscaling = true;
        persist();
        // No program can read the NVIDIA app's switch (the SDK's advice): said once, here.
        shown(plugin.osd.show("AI upscaling: on (Super Resolution must also be on in the NVIDIA app)"));
      } else {
        look.render.upscaling = false;
        persist();
        upscaling = await plugin.video.upscaling.set(false).catch(() => upscaling);
        shown(plugin.osd.show("AI upscaling: off"));
      }
    } finally {
      upscalingBusy = false;
      if (!stopped) {
        plugin.update();
        followPanel(panel.state);
      }
    }
  };
  runs.set("video.upscaling", switchUpscaling);
  commands.add({
    id: "video.upscaling",
    title: "AI Upscaling",
    category: category.picture,
    ...inPicture,
    order: 461,
    group: "video-rendering",
    checked: () => look.render.upscaling,
    enabled: () => upscalingBusy || look.render.upscaling || !upscalingBlocked(),
    reason: upscalingReason,
    busy: () => upscalingBusy,
    run: switchUpscaling,
  });
  // Saved on: ask again for this run (this checks the GPU, which the user wants on).
  if (look.render.upscaling && !softwareNow())
    void plugin.video.upscaling.set(true).then(
      (state) => {
        upscaling = state;
        if (stopped) return;
        plugin.update();
        followPanel(panel.state);
      },
      () => void readUpscaling(),
    );
  // While the panel shows and the state has been read: its status follows the player (the factor,
  // the reason, the decoder). Shown again while upscaling is saved on, the state is read afresh.
  let following: Disposable | null = null;
  function followPanel(state: PanelState | null) {
    const visible = state?.effective === true && !softwareNow() && !stopped;
    if (visible && following === null && upscaling !== null) {
      following = plugin.video.upscaling.onChange((next) => {
        upscaling = next;
        plugin.update();
      });
      if (look.render.upscaling) void readUpscaling();
    } else if (!visible && following !== null) {
      following.dispose();
      following = null;
    }
  }
  panel.onChange(followPanel);
  followPanel(panel.state);
  let renderingSource = player.state.sourceId;
  player.onChange((state) => {
    if (state.sourceId === renderingSource) return;
    renderingSource = state.sourceId;
    void refreshRendering();
  });
  await showRendering();
}

/**
 * The panel's status line, as data the panel words: `status` names a failure ("" when none), the
 * detail in a tooltip is the player's message, or `conflict` when another plugin owns the layer. The
 * chain first, then the display layer, then the render options. Software rendering is not a failure
 * here: the GPU controls are greyed with the reason instead.
 */
function statusLine(filter: FilterStatus, view: FilterStatus, render: FilterStatus) {
  if (filter.state === "failed")
    return { failed: true, status: filter.error.code === "unsupported" ? "no-picture" : "filter", detail: filter.error.message, conflict: false };
  if (view.state === "failed") return { failed: true, status: "view", ...reason(view.error) };
  if (render.state === "failed" && render.error.code !== "unsupported")
    return {
      failed: true,
      status: shaderFailed(render.error) ? "shader" : render.error.code === "conflict" ? "render-conflict" : "render",
      ...reason(render.error),
    };
  return { failed: false, status: "", detail: "", conflict: false };
}

/** mpv compiles a shader on first use; the player pushes this later (it does not name the file). */
function shaderFailed(error: PluginError) {
  return error.code === "failed" && error.message.startsWith("shader compile failed");
}

/** Why the display layer (比例, 帧位) or the render options refused; `conflict`: another plugin owns them. */
function reason(error: PluginError) {
  return error.code === "conflict" ? { detail: "", conflict: true } : { detail: error.message, conflict: false };
}

/** `settings.json` through `plugin.storage`; without a data directory, settings last for this run. */
function settingsStore(plugin: Plugin) {
  try {
    return plugin.storage.json<Record<string, unknown>>("settings", {
      defaults: {},
      validate: (value): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value),
    });
  } catch {
    let value: Record<string, unknown> = {};
    return {
      get value() {
        return value;
      },
      save(next: Record<string, unknown>) {
        value = next;
      },
    };
  }
}
