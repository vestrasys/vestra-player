import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import type { FilterStatus, Plugin, PluginError, RenderingPreference, RenderingState, Scope } from "@vestra-player/sdk";
import {
  DEINTERLACE_MODES,
  FITS,
  MAX_SHADERS,
  PAN_STEP,
  TONE_MAPPINGS,
  ZOOM_STEP,
  DEINTERLACE_OUTPUTS,
  DENOISE_MODES,
  POSITION_STEP,
  RATIOS,
  REFRAME_MODES,
  ROTATIONS,
  TABS,
  aspectIndex,
  aspectMessage,
  blendFps,
  centred,
  clamp,
  defaultEffects,
  defaultLook,
  defaults,
  droppedTooMany,
  isHdr,
  isShaderName,
  nextAspect,
  nodesFor,
  parse,
  parseAspect,
  peakOf,
  presets,
  reframeMessage,
  serialize,
  upright,
} from "./model.ts";
import type { DeinterlaceMode, DeinterlaceOutput, Fit, Frame, FrameCount, Look, ReframeMode, Rotation, Toggle, ToneMapping } from "./model.ts";

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
type Strength = "sharpen" | "edge" | "denoise" | "deblock" | "deband";
const strengths: readonly Strength[] = ["sharpen", "edge", "denoise", "deblock", "deband"];
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
/** Deinterlace Auto is the player's deinterlacer, always at field rate: Output is greyed (menu reason; the window has the same tip). */
const AUTO_OUTPUT = "Auto outputs by field (double rate); this setting applies to Always";
/** The renderer switch (render modes §8.4): its title, and the title while a restart would change the renderer. */
const GPU_TITLE = "GPU Rendering (Experimental)";
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
 * Picture tool: one four-page window, one owned video filter, the display layer
 * (aspect, fit, zoom, pan) and the 视频 ▸ menu. Colour, effects, deinterlace, crop/pad,
 * aspect and fit are global (`settings.json`); flip, rotation, zoom and pan belong to
 * the current source and reset when it changes.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const store = settingsStore(plugin);
  const { tab: savedTab, ...saved } = parse(store.value);
  const look: Look = { ...defaultLook(), ...saved };
  let tab = savedTab;
  let enabled = true;
  /** E13 zoom and pan: per source, never saved. */
  let frame: Frame = centred();

  const filter = plugin.video.filter("picture");
  const view = plugin.video.view();
  const window = plugin.windows.create("picture.slint");
  const aspectWindow = plugin.windows.create("ui/aspect.slint");
  const shadersWindow = plugin.windows.create("ui/shaders.slint");
  const persist = () => store.save(serialize(look, tab));

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
    // A compile error arrives after `applied`, when the window is usually closed: always said.
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
    }
    if (transferRead || reading || player.state.phase !== "ready") return;
    reading = true;
    try {
      for (let attempt = 0; attempt < MEASURE_TRIES && !stopped && player.state.sourceId === source; attempt++) {
        if (attempt > 0) await pause(MEASURE_DELAY_MS);
        const stats = await player.stats().catch(() => null);
        if (stopped || player.state.sourceId !== source) return;
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
  const toggleOf = (field: Strength): Toggle =>
    field === "denoise" ? effects.denoise[effects.denoise.shown] : effects[field];

  // ---------------------------------------------------------------- window
  window.bind(() => {
    const denoise = effects.denoise[effects.denoise.shown];
    return {
      ...look.picture,
      sharpenEnabled: effects.sharpen.on,
      sharpenStrength: effects.sharpen.strength,
      edgeEnabled: effects.edge.on,
      edgeStrength: effects.edge.strength,
      denoiseIndex: DENOISE_MODES.indexOf(effects.denoise.shown),
      denoiseEnabled: denoise.on,
      denoiseStrength: denoise.strength,
      deblockEnabled: effects.deblock.on,
      deblockStrength: effects.deblock.strength,
      debandEnabled: effects.deband.on,
      debandStrength: effects.deband.strength,
      blendEnabled: effects.frameBlend,
      blendEnough: blendEnough(),
      customRatio: look.display.customAspect,
      aspectIndex: aspectIndex(look.display),
      fitIndex: FITS.findIndex((f) => f.id === look.display.fit),
      zoomLevel: Math.log2(frame.zoom),
      zoomPercent: percent(frame.zoom),
      flipV: look.geometry.flipV,
      flipH: look.geometry.flipH,
      rotateIndex: ROTATIONS.indexOf(look.geometry.rotate),
      reframeIndex: REFRAME_MODES.indexOf(look.reframe.mode),
      ratioIndex: look.reframe.ratio,
      deinterlaceIndex: DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode),
      outputIndex: DEINTERLACE_OUTPUTS.findIndex((m) => m.id === look.deinterlace.output),
      renderReady: gpuReady(),
      toneIndex: TONE_MAPPINGS.findIndex((t) => t.id === look.render.toneMapping),
      targetPeak: look.render.targetPeak,
      peakNits: look.render.targetPeak,
      notHdr: notHdr(),
      shadersEnabled: look.render.shadersEnabled,
      shaderCount: look.render.shaders.length,
      interpolation: look.render.interpolation,
      tabIndex: tab,
      effectEnabled: enabled,
      ...statusLine(filter.status, view.status, render.status),
    };
  });
  /** Store a slider value; how it takes effect (null when the field is unknown). */
  const storeValue = (field: string, value: number) => {
    if (!Number.isFinite(value)) return null;
    if (field === "blurStrength") look.picture.blurStrength = clamp(value, 0, 10);
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = clamp(value, -100, 100);
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).strength = clamp(Math.round(value), 0, 100);
    else if (field === "zoom") frame.zoom = zoomOf(2 ** clamp(value, -2, 2));
    else if (field === "targetPeak") look.render.targetPeak = peakOf(value);
    else return null;
    return field === "zoom" ? "view" : field === "targetPeak" ? "render" : (strengths as string[]).includes(field) ? "rebuild" : "live";
  };
  // eq, hue and gblur change in place while playing; the other filters rebuild, so they wait for release.
  // Zoom (a display property) and 目标亮度 (a render option) follow the drag. The first move away
  // from the defaults adds the eq node (one rebuild); from then on the drag updates it in place,
  // also through 0, and the release removes it when every colour is back at its default.
  window.on("adjust", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "view") showView();
    else if (kind === "render") renderChanged();
    else if (kind === "live") change(true);
  });
  window.on("commit", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "view") showView();
    else if (kind === "render") renderChanged();
    else if (kind) change();
  });
  window.on("resetField", (field: string) => {
    if (field === "zoom") {
      frame = centred();
      showView();
      return;
    }
    if (field === "blurStrength") look.picture.blurStrength = defaults.blurStrength;
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = defaults[field as Color];
    else if (field === "denoise") toggleOf("denoise").strength = defaultEffects().denoise[effects.denoise.shown].strength;
    else if ((strengths as string[]).includes(field))
      toggleOf(field as Strength).strength = (defaultEffects()[field as Exclude<Strength, "denoise">] as Toggle).strength;
    else return;
    change();
  });
  window.on("preset", (name: string) => {
    const preset = presets[name];
    if (!preset) return;
    look.picture = { ...preset };
    change();
  });
  window.on("toggleEffect", (on: boolean) => {
    enabled = on === true;
    apply();
    if (wantsMeasure()) void measure();
  });
  window.on("toggle", (field: string, on: boolean) => {
    const value = on === true;
    if (field === "blur") look.picture.blurEnabled = value;
    else if (field === "frameBlend") return setBlend(value, false);
    else if (field === "interpolation") return setInterpolation(value, false);
    else if (field === "shaders") return setShaders(value, false);
    else if (field === "flipV" || field === "flipH") look.geometry[field] = value;
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).on = value;
    else return;
    change();
  });
  window.on("choose", (field: string, index: number) => {
    if (!Number.isInteger(index) || index < 0) return;
    if (field === "denoiseMode" && index < DENOISE_MODES.length) {
      effects.denoise.shown = DENOISE_MODES[index]!;
      persist();
      return;
    }
    if (field === "rotate" && index < ROTATIONS.length) look.geometry.rotate = ROTATIONS[index]!;
    else if (field === "reframe" && index < REFRAME_MODES.length) look.reframe.mode = REFRAME_MODES[index]!;
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
  window.on("customAspect", () => openCustom());
  window.on("shaderList", () => openShaders());
  window.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    persist();
  });
  window.on("retry", () => {
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

  /** 画面调节…; HDR 设置… opens it on the 高级 page. */
  const showWindow = (page?: number) => {
    if (page !== undefined && page !== tab) {
      tab = page;
      persist();
      plugin.update();
    }
    return window.show();
  };

  // ---------------------------------------------------------------- actions (menu and keys)
  const reset = () => {
    look.picture = { ...defaults };
    change();
    shown(plugin.osd.show("Picture reset"));
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
    toggle.on = !toggle.on;
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

  // ---------------------------------------------------------------- menu 视频 ▸ (@video)
  const { commands } = plugin;
  // 402 像素着色 (E17, E16): GPU render options, disabled with the reason under software rendering.
  // The list and the folder stay usable, so shaders can be arranged beforehand.
  const gpu = { enabled: gpuReady, reason: gpuReason };
  const pixel = { order: 402 } as const;
  commands.add({
    id: "video.shader.use",
    path: ["@video", "Pixel Shaders", "Use Shaders"],
    ...pixel,
    group: "video-source",
    bindings: ["Shift+P"],
    checked: () => look.render.shadersEnabled,
    ...gpu,
    run: () => setShaders(!look.render.shadersEnabled, true),
  });
  commands.add({ id: "video.shader.list", path: ["@video", "Pixel Shaders", "Shader List…"], ...pixel, group: "video-source", run: openShaders });
  commands.add({ id: "video.shader.folder", path: ["@video", "Pixel Shaders", "Open Shader Folder"], ...pixel, group: "video-source", run: openShaderFolder });
  commands.add({ id: "video.shader.reload", path: ["@video", "Pixel Shaders", "Reload Shaders"], ...pixel, group: "video-source", ...gpu, run: reloadShaders });
  for (const { id, label } of TONE_MAPPINGS)
    commands.add({
      id: `video.hdr.${id}`,
      path: ["@video", "Pixel Shaders", "HDR Tone Mapping", label],
      ...pixel,
      group: "video-hdr",
      radio: true,
      checked: () => look.render.toneMapping === id,
      ...gpu,
      run: () => setTone(id),
    });
  commands.add({
    id: "video.hdr.peak",
    path: ["@video", "Pixel Shaders", "HDR Tone Mapping", "Dynamic Peak Detection"],
    ...pixel,
    group: "video-hdr-peak",
    checked: () => look.render.peakDetect,
    ...gpu,
    run: switchPeak,
  });
  commands.add({ id: "video.hdr.settings", path: ["@video", "Pixel Shaders", "HDR Settings…"], ...pixel, group: "video-hdr", ...gpu, run: () => showWindow(3) });
  // 403 反交错 (E10): Auto is the player's per-frame deinterlacer at field rate, Always is BWDIF
  // with the Output setting (greyed under Auto).
  commands.add({
    id: "video.deinterlace.cycle",
    path: ["@video", "Deinterlace", "Cycle"],
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
      path: ["@video", "Deinterlace", label],
      order: 403,
      group: "video-source",
      radio: true,
      checked: () => look.deinterlace.mode === id,
      run: () => setDeinterlace(id),
    });
  for (const { id, label } of DEINTERLACE_OUTPUTS)
    commands.add({
      id: `video.deinterlace.output.${id}`,
      path: ["@video", "Deinterlace", "Output", label],
      order: 403,
      group: "video-deinterlace-output",
      radio: true,
      enabled: () => look.deinterlace.mode !== "auto",
      reason: () => (look.deinterlace.mode === "auto" ? AUTO_OUTPUT : null),
      checked: () => look.deinterlace.output === id,
      run: () => setOutput(id),
    });
  // 404 比例 (E11): display mode, then the aspect ratios.
  for (const { id, menu } of FITS)
    commands.add({
      id: `video.fit.${id}`,
      path: ["@video", "Aspect", menu],
      order: 404,
      group: "video-display",
      radio: true,
      checked: () => look.display.fit === id,
      run: () => setFit(id),
    });
  commands.add({
    id: "video.aspect.cycle",
    path: ["@video", "Aspect", "Cycle Aspect Ratio"],
    order: 404,
    group: "video-aspect",
    bindings: ["Shift+F6"],
    run: () => setAspect(nextAspect(look.display)),
  });
  commands.add({
    id: "video.aspect.default",
    path: ["@video", "Aspect", "Default (Source)"],
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
      path: ["@video", "Aspect", label],
      order: 404,
      group: "video-aspect",
      radio: true,
      checked: () => aspectIndex(look.display) === i + 1,
      run: () => setAspect(aspect),
    }),
  );
  // The 自定义 item shows its ratio in the title, so it is re-added (with 自定义比例… after it) when that changes.
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
      order: 404,
      group: "video-aspect",
      radio: true,
      checked: () => aspectIndex(look.display) === RATIOS.length + 1,
      run: () => setAspect(parseAspect(text)),
    };
    if (text)
      custom.commands.add({
        ...item,
        path: ["@video", "Aspect", plugin.i18n.text("Custom ({})", [text])],
      });
    else custom.commands.add({ ...item, path: ["@video", "Aspect", "Custom (x:y)"], enabled: false, reason: "Enter one with Custom Aspect Ratio… first" });
    custom.commands.add({ id: "video.aspect.edit", path: ["@video", "Aspect", "Custom Aspect Ratio…"], order: 404, group: "video-aspect", run: openCustom });
  };
  addCustom();
  // 405 裁剪/拉伸 (E12).
  commands.add({
    id: "video.reframe.none",
    path: ["@video", "Crop/Stretch", "None"],
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
        path: ["@video", "Crop/Stretch", mode === "pad" ? row.pad : row.crop],
        order: 405,
        group: "video-display",
        radio: true,
        checked: () => look.reframe.mode === mode && look.reframe.ratio === ratio,
        run: () => setReframe(mode, ratio),
      }),
    );
  const positions = [
    { id: "reset", title: "Reset Position", dx: 0, dy: 0 },
    { id: "left", title: "Move Left", dx: -POSITION_STEP, dy: 0 },
    { id: "right", title: "Move Right", dx: POSITION_STEP, dy: 0 },
    { id: "up", title: "Move Up", dx: 0, dy: -POSITION_STEP },
    { id: "down", title: "Move Down", dx: 0, dy: POSITION_STEP },
  ] as const;
  for (const { id, title, dx, dy } of positions)
    commands.add({
      id: `video.reframe.${id}`,
      path: ["@video", "Crop/Stretch", title],
      order: 405,
      group: "video-reframe-position",
      repeat: id !== "reset",
      run: () => move(dx, dy),
    });
  // 406 帧位 (E13): keypad keys plus mpv-style Alt keys for keyboards without one.
  const frames = [
    { id: "reset", title: "Reset Framing", group: "video-display", bindings: ["Numpad5", "Alt+Backspace"], run: () => resetFrame(false), repeat: false },
    { id: "zoom-in", title: "Zoom In", group: "video-display", bindings: ["Numpad9", "Alt+Plus"], run: () => zoomBy(ZOOM_STEP), repeat: true },
    { id: "zoom-out", title: "Zoom Out", group: "video-display", bindings: ["Numpad1", "Alt+-"], run: () => zoomBy(1 / ZOOM_STEP), repeat: true },
    { id: "right", title: "Pan →", group: "video-frame-pan", bindings: ["Ctrl+Numpad6", "Alt+ArrowRight"], run: () => panBy(PAN_STEP, 0, PAN.right), repeat: true },
    { id: "left", title: "Pan ←", group: "video-frame-pan", bindings: ["Ctrl+Numpad4", "Alt+ArrowLeft"], run: () => panBy(-PAN_STEP, 0, PAN.left), repeat: true },
    { id: "up", title: "Pan ↑", group: "video-frame-pan", bindings: ["Ctrl+Numpad8", "Alt+ArrowUp"], run: () => panBy(0, -PAN_STEP, PAN.up), repeat: true },
    { id: "down", title: "Pan ↓", group: "video-frame-pan", bindings: ["Ctrl+Numpad2", "Alt+ArrowDown"], run: () => panBy(0, PAN_STEP, PAN.down), repeat: true },
    { id: "centre", title: "Center Frame", group: "video-frame-pan", bindings: ["Ctrl+Numpad5"], run: () => resetFrame(true), repeat: false },
  ] as const;
  for (const { id, title, group, bindings, run, repeat } of frames)
    commands.add({ id: `video.frame.${id}`, path: ["@video", "Framing", title], order: 406, group, bindings: [...bindings], repeat, run });
  // 408 图像处理 (E04–E06, E08; 高斯模糊 moved here, same id and key).
  commands.add({
    id: "video.flip.vertical",
    path: ["@video", "Image Processing", "Flip Vertically"],
    order: 408,
    group: "video-effects",
    bindings: ["Ctrl+P"],
    checked: () => look.geometry.flipV,
    run: () => flip("flipV"),
  });
  commands.add({
    id: "video.flip.horizontal",
    path: ["@video", "Image Processing", "Flip Horizontally"],
    order: 408,
    group: "video-effects",
    bindings: ["Ctrl+Z"],
    checked: () => look.geometry.flipH,
    run: () => flip("flipH"),
  });
  commands.add({
    id: "video.gaussian",
    path: ["@video", "Image Processing", "Gaussian Blur"],
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
      path: ["@video", "Image Processing", row.title],
      order: 408,
      group: row.group,
      bindings: [...row.bindings],
      checked: () => toggle().on,
      run: () => switchEffect(toggle(), row),
    });
  }
  // E18: 倍帧 adds frameBlend once the source rate is known; 运动补偿 is a GPU render option.
  commands.add({
    id: "video.frame-blend",
    path: ["@video", "Image Processing", "Frame Doubling"],
    order: 408,
    group: "video-frames",
    checked: () => effects.frameBlend,
    run: () => setBlend(!effects.frameBlend, true),
  });
  commands.add({
    id: "video.interpolation",
    path: ["@video", "Image Processing", "Motion Interpolation"],
    order: 408,
    group: "video-frames",
    checked: () => look.render.interpolation,
    ...gpu,
    run: () => setInterpolation(!look.render.interpolation, true),
  });
  // 409 图像旋转 (E09).
  commands.add({
    id: "video.rotate.cycle",
    path: ["@video", "Rotation", "Cycle"],
    order: 409,
    group: "video-effects",
    bindings: ["Alt+K"],
    run: () => rotate(ROTATIONS[(ROTATIONS.indexOf(look.geometry.rotate) + 1) % ROTATIONS.length]!),
  });
  for (const { degrees, title } of ROTATION_TITLES)
    commands.add({
      id: `video.rotate.${degrees}`,
      path: ["@video", "Rotation", title],
      order: 409,
      group: "video-effects",
      radio: true,
      checked: () => look.geometry.rotate === degrees,
      run: () => rotate(degrees),
    });
  // 440–449 the window and the colour keys.
  commands.add({ id: "video.picture", path: ["@video", "Picture Adjustments…"], order: 440, group: "video-color", run: () => showWindow() });
  commands.add({ id: "video.reset", path: ["@video", "Reset"], order: 441, group: "video-color", bindings: ["Q"], run: reset });
  for (const { field, keys, order, down, up } of steps) {
    // All colour steps repeat while held.
    commands.add({
      id: `video.${field}.down`,
      path: ["@video", down.title],
      order,
      group: "video-color",
      bindings: [keys[0]],
      repeat: true,
      run: () => step(down, field, -1),
    });
    commands.add({
      id: `video.${field}.up`,
      path: ["@video", up.title],
      order: order + 1,
      group: "video-color",
      bindings: [keys[1]],
      repeat: true,
      run: () => step(up, field, 1),
    });
  }
  apply();
  applyRender();

  // 460 GPU Rendering (Experimental): the player's renderer choice (`plugin.rendering`). The player
  // keeps the choice and applies it at the next start; this item only shows and changes it. A
  // title cannot change in place, so the item lives in a child scope rebuilt when it changes.
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
      path: ["@video", title],
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
        path: ["@video", "Restart Player to Apply"],
        order: 460,
        group: "video-rendering",
        busy: () => restarting || switching,
        run: restartNow,
      });
  };
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
    try {
      await plugin.rendering.restartPlayer();
    } catch (error) {
      if (!stopped) shown(plugin.osd.show("Could not restart the player: {}", [(error as PluginError).message]));
    } finally {
      restarting = false;
    }
  };
  let renderingSource = player.state.sourceId;
  player.onChange((state) => {
    if (state.sourceId === renderingSource) return;
    renderingSource = state.sourceId;
    void refreshRendering();
  });
  await showRendering();
}

/**
 * The window's status line, as data the window words: `status` names a failure ("" when none), the
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
