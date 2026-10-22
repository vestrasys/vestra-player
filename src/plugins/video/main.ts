import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import type { FilterStatus, Plugin, PluginError } from "@vestra-player/sdk";
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
  aspectLabel,
  blendFps,
  centred,
  clamp,
  defaultEffects,
  defaultLook,
  defaults,
  denoiseLabels,
  droppedTooMany,
  isHdr,
  isShaderName,
  nextAspect,
  nodesFor,
  parse,
  parseAspect,
  peakOf,
  presets,
  reframeLabel,
  serialize,
  upright,
} from "./model.ts";
import type { DeinterlaceMode, DeinterlaceOutput, Fit, Frame, FrameCount, Look, ReframeMode, Rotation, Toggle, ToneMapping } from "./model.ts";

type Color = "brightness" | "contrast" | "saturation" | "hue" | "gamma";
const colors: readonly Color[] = ["brightness", "contrast", "saturation", "hue", "gamma"];
/** Keyboard steps (W/E, R/T, Y/U; 色调 I/O): label, field, keys for −1 and +1, menu order. */
const steps: readonly [string, Color, string, string, number][] = [
  ["亮度", "brightness", "W", "E", 442],
  ["对比度", "contrast", "R", "T", 444],
  ["饱和度", "saturation", "Y", "U", 446],
  ["色调", "hue", "I", "O", 448],
];
/** Effect rows with one 0–100 strength; their filters rebuild, so a drag commits on release. */
type Strength = "sharpen" | "edge" | "denoise" | "deblock" | "deband";
const strengths: readonly Strength[] = ["sharpen", "edge", "denoise", "deblock", "deband"];
/** E16, E17 and 运动补偿 are GPU render options; software rendering (and Windows for now) refuses them. */
const SOFTWARE = "需要 GPU 渲染（当前为软件渲染）";
const NOT_HDR = "当前片源不是 HDR";
/** E18: the source rate is read this long after the chain without frameBlend is in place, this many times at most. */
const MEASURE_DELAY_MS = 400;
const MEASURE_TRIES = 8;
const ASPECT_FORMAT = "格式：16:9 或 2.39";
const percent = (n: number) => `${Math.round(n * 100)}%`;
const signed = (n: number) => (n > 0 ? `+${n}` : `${n}`);
const onOff = (on: boolean) => (on ? "开" : "关");
/** A timer that does not keep the plugin process alive. */
const pause = (ms: number) => new Promise<void>((done) => void setTimeout(done, ms).unref?.());

/**
 * Picture tool: one four-page window, one owned video filter, the display layer
 * (aspect, fit, zoom, pan) and the 视频 ▶ menu. Colour, effects, deinterlace, crop/pad,
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
  // 启用画面效果 leaves them alone. Under software rendering every set is refused (`unsupported`).
  const render = plugin.video.render();
  const shaderFolder = join(plugin.dataDirectory, "shaders");
  const shaderPath = (name: string) => join(shaderFolder, name);
  const gpuReady = () => !(render.status.state === "failed" && render.status.error.code === "unsupported");
  const gpuReason = () => (gpuReady() ? null : SOFTWARE);
  let renderSent = "";
  /** Set by a user action, so a conflict at activation stays off screen (as with the display layer). */
  let renderAnnounce = false;
  /** Missing shader files are left out (the player refuses them); 重新载入着色器 drops them from the list. */
  const applyRender = (user = false, force = false) => {
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
    if (shaderFailed(status.error)) osd("着色器载入失败");
    else if (announce && status.error.code !== "unsupported") osd(`渲染选项不可用：${renderReason(status.error)}`);
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
  // blended rate), once per source and deinterlace setting (field output doubles the rate).
  const blendKey = () => `${player.state.sourceId}:${look.deinterlace.mode}:${look.deinterlace.output}`;
  let measured: { key: string; fps: number | null } | null = null;
  let abandoned: string | null = null;
  const blendWanted = () => enabled && look.effects.frameBlend;
  const blendRate = () => (blendWanted() && measured?.key === blendKey() ? blendFps(measured.fps) : null);
  const blendEnough = () => blendWanted() && measured?.key === blendKey() && blendFps(measured.fps) === null;

  let sent = "";
  let sentBlend: number | null = null;
  /** The SDK keeps one change in flight; an unchanged chain is not resent unless it failed. */
  const apply = () => {
    sentBlend = blendRate();
    const nodes = enabled ? nodesFor(look, sentBlend) : [];
    const key = JSON.stringify(nodes);
    if (key === sent && filter.status.state !== "failed") return;
    sent = key;
    filter.set(nodes);
  };
  const change = () => {
    apply();
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
  const osd = (text: string) => void plugin.osd.show(text).catch(() => undefined);
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
      blendNote: blendEnough() ? "当前帧率已足够" : "",
      aspectNames: [
        "默认（按来源）",
        ...RATIOS.map((r) => r.label),
        ...(look.display.customAspect ? [`自定义（${look.display.customAspect}）`] : []),
      ],
      aspectIndex: aspectIndex(look.display),
      fitIndex: FITS.findIndex((f) => f.id === look.display.fit),
      zoomLevel: Math.log2(frame.zoom),
      zoomText: `缩放 ${percent(frame.zoom)}`,
      flipV: look.geometry.flipV,
      flipH: look.geometry.flipH,
      rotateIndex: ROTATIONS.indexOf(look.geometry.rotate),
      reframeIndex: REFRAME_MODES.indexOf(look.reframe.mode),
      ratioIndex: look.reframe.ratio,
      deinterlaceIndex: DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode),
      outputIndex: DEINTERLACE_OUTPUTS.findIndex((m) => m.id === look.deinterlace.output),
      renderReady: gpuReady(),
      renderReason: SOFTWARE,
      toneIndex: TONE_MAPPINGS.findIndex((t) => t.id === look.render.toneMapping),
      targetPeak: look.render.targetPeak,
      hdrNote: `目标亮度 ${look.render.targetPeak} 尼特${gpuReady() && notHdr() ? ` · ${NOT_HDR}` : ""}`,
      shadersEnabled: look.render.shadersEnabled,
      shaderCount: `${look.render.shaders.length} 个着色器`,
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
  // Zoom (a display property) and 目标亮度 (a render option) follow the drag.
  window.on("adjust", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "view") showView();
    else if (kind === "render") renderChanged();
    else if (kind === "live") change();
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
    osd(`画面显示不可用：${viewReason(status.error)}`);
  });
  const displayChanged = (user = false) => {
    persist();
    showView(user);
  };
  const setAspect = (aspect: number | null) => {
    look.display.aspect = aspect;
    displayChanged(true);
    osd(`宽高比：${aspectLabel(look.display)}`);
  };
  const setFit = (fit: Fit) => {
    look.display.fit = fit;
    displayChanged(true);
    osd(FITS.find((f) => f.id === fit)!.label);
  };
  const zoomOf = (zoom: number) => clamp(Math.round(zoom * 10000) / 10000, 0.25, 4);
  const zoomBy = (factor: number) => {
    frame.zoom = zoomOf(frame.zoom * factor);
    showView(true);
    osd(`帧位调整：${factor > 1 ? "增加" : "减少"}尺寸 (${percent(frame.zoom)})`);
  };
  const panBy = (dx: number, dy: number, direction: string) => {
    const pan = (n: number, d: number) => clamp(Math.round((n + d) * 100) / 100, -1, 1);
    frame.panX = pan(frame.panX, dx);
    frame.panY = pan(frame.panY, dy);
    showView(true);
    osd(`帧位调整：向${direction}位移 (${percent(frame.panX)}, ${percent(frame.panY)})`);
  };
  const resetFrame = (centreOnly: boolean) => {
    frame = centreOnly ? { ...frame, panX: 0, panY: 0 } : centred();
    showView(true);
    osd(centreOnly ? "帧位调整：位移到中间" : "帧位调整：重置");
  };
  showView();

  // 自定义比例… (E11): a small input window; 确定 only takes a valid ratio.
  let draft = "";
  aspectWindow.bind(() => {
    const valid = parseAspect(draft) !== null;
    return { valid, error: draft.trim() === "" || valid ? "" : ASPECT_FORMAT };
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
  const offNote = () => (enabled ? "" : "（画面效果已关闭）");
  const reset = () => {
    look.picture = { ...defaults };
    change();
    osd("画面调节已复位");
  };
  const step = (label: string, field: Color, delta: number) => {
    look.picture[field] = clamp(Math.round(look.picture[field]) + delta, -100, 100);
    change();
    osd(`${label} ${delta > 0 ? "+" : "-"}1% (${signed(look.picture[field])}%)${offNote()}`);
  };
  const flip = (field: "flipV" | "flipH", label: string) => {
    look.geometry[field] = !look.geometry[field];
    apply();
    osd(`${label}: ${onOff(look.geometry[field])}`);
  };
  const rotate = (degrees: Rotation) => {
    look.geometry.rotate = degrees;
    apply();
    osd(`图像旋转: ${degrees === 0 ? "不旋转" : `${degrees}°`}`);
  };
  const switchEffect = (toggle: Toggle, label: string) => {
    toggle.on = !toggle.on;
    change();
    osd(`${label}: ${onOff(toggle.on)}`);
  };
  const setDeinterlace = (mode: DeinterlaceMode) => {
    look.deinterlace.mode = mode;
    change();
    osd(`反交错: ${DEINTERLACE_MODES.find((m) => m.id === mode)!.label}`);
  };
  const setOutput = (output: DeinterlaceOutput) => {
    look.deinterlace.output = output;
    change();
    osd(`反交错输出: ${DEINTERLACE_OUTPUTS.find((m) => m.id === output)!.label}`);
  };
  const setReframe = (mode: ReframeMode, ratio = look.reframe.ratio) => {
    look.reframe.mode = mode;
    look.reframe.ratio = ratio;
    change();
    osd(`裁剪/拉伸: ${reframeLabel(look.reframe)}`);
  };
  const move = (dx: number, dy: number) => {
    const r = look.reframe;
    r.x = dx === 0 && dy === 0 ? 50 : clamp(r.x + dx, 0, 100);
    r.y = dx === 0 && dy === 0 ? 50 : clamp(r.y + dy, 0, 100);
    change();
    osd(`裁剪位置 (${r.x}%, ${r.y}%)`);
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
    if (menu || excluded) osd(`倍帧: ${onOff(on)}${excluded ? "（已关闭运动补偿）" : on ? offNote() : ""}`);
  };
  const setInterpolation = (on: boolean, menu: boolean) => {
    look.render.interpolation = on;
    const excluded = on && effects.frameBlend;
    if (excluded) {
      switchBlend(false);
      apply();
    }
    renderChanged(true);
    if (menu || excluded) osd(`运动补偿: ${onOff(on)}${excluded ? "（已关闭倍帧）" : ""}`);
  };
  // E16: the options only act on HDR sources; on a known SDR source the OSD says so.
  const setTone = (id: ToneMapping) => {
    look.render.toneMapping = id;
    renderChanged(true);
    osd(`HDR 色调映射: ${TONE_MAPPINGS.find((t) => t.id === id)!.label}${notHdr() ? `（${NOT_HDR}）` : ""}`);
  };
  const switchPeak = () => {
    look.render.peakDetect = !look.render.peakDetect;
    renderChanged(true);
    osd(`动态峰值检测: ${onOff(look.render.peakDetect)}`);
  };
  // E17: the list is file names in `<dataDirectory>/shaders/`; the switch keeps the list.
  const setShaders = (on: boolean, menu: boolean) => {
    look.render.shadersEnabled = on;
    renderChanged(true);
    const count = look.render.shaders.length;
    if (menu) osd(!on ? "像素着色: 关" : count === 0 ? "像素着色: 没有着色器" : `像素着色: 开 (${count})`);
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
    { active: () => enabled && look.deinterlace.mode !== "off" && look.deinterlace.output === "field", text: "反交错按场输出丢帧较多，可改为按帧" },
    { active: () => sentBlend !== null, text: "倍帧丢帧较多，建议关闭" },
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
    const now = { source: state.sourceId, shown: stats.framesShown, dropped: stats.framesDropped, watching: watching.map((h) => h.text).join() };
    if (counted && counted.source === now.source && counted.watching === now.watching && droppedTooMany(counted, now)) {
      for (const h of watching) h.warned = now.source;
      osd(watching.map((h) => h.text).join("；"));
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
      osd(`最多 ${MAX_SHADERS} 个着色器`);
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
    const path = await plugin.dialogs.openFile({ title: "添加着色器（.glsl）" }).catch(() => null);
    if (path === null) return;
    const name = basename(path);
    if (!isShaderName(name)) return osd("只能添加 .glsl 或 .hook 着色器文件");
    if (!look.render.shaders.includes(name) && look.render.shaders.length >= MAX_SHADERS) return osd(`最多 ${MAX_SHADERS} 个着色器`);
    try {
      await mkdir(shaderFolder, { recursive: true });
      if (resolve(path) !== resolve(shaderPath(name))) await copyFile(path, shaderPath(name));
    } catch (error) {
      return osd(`着色器复制失败：${error instanceof Error ? error.message : String(error)}`);
    }
    addShaderName(name);
  };
  const openShaderFolder = async () => {
    try {
      await mkdir(shaderFolder, { recursive: true });
      await plugin.system.openPath(shaderFolder);
    } catch (error) {
      osd(`无法打开着色器文件夹：${error instanceof Error ? error.message : String(error)}`);
    }
  };
  /** 重新载入着色器: rescan the folder, drop names whose file is gone, submit the list again. */
  const reloadShaders = async () => {
    const files = await refreshShaders();
    const missing = look.render.shaders.filter((name) => !files.includes(name));
    editShaders((list) => list.splice(0, list.length, ...list.filter((name) => files.includes(name))), true);
    osd(missing.length > 0 ? `已移除不存在的着色器：${missing.join("、")}` : `已重新载入着色器 (${look.render.shaders.length})`);
  };
  const openShaders = () => {
    void refreshShaders();
    void shadersWindow.show();
  };
  shadersWindow.bind(() => ({
    names: look.render.shaders.map((name) => (existsSync(shaderPath(name)) ? name : `${name}（文件不存在）`)),
    selected: picked,
    available,
    heading: `已启用 ${look.render.shaders.length}/${MAX_SHADERS}（按顺序执行）`,
    note: !gpuReady() ? SOFTWARE : look.render.shadersEnabled ? "" : "「使用着色器」未打开",
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

  // ---------------------------------------------------------------- menu 视频 ▶
  const { commands } = plugin;
  // 402 像素着色 (E17, E16): GPU render options, disabled with the reason under software rendering.
  // The list and the folder stay usable, so shaders can be arranged beforehand.
  const gpu = { enabled: gpuReady, reason: gpuReason };
  const pixel = (id: string, path: string[], group: string, extra: Omit<Parameters<typeof commands.add>[0], "id" | "path" | "order" | "group">) =>
    commands.add({ id, path: ["视频", "像素着色", ...path], order: 402, group, ...extra });
  pixel("video.shader.use", ["使用着色器"], "video-source", { bindings: ["Shift+P"], checked: () => look.render.shadersEnabled, ...gpu, run: () => setShaders(!look.render.shadersEnabled, true) });
  pixel("video.shader.list", ["着色器列表…"], "video-source", { run: openShaders });
  pixel("video.shader.folder", ["打开着色器文件夹"], "video-source", { run: openShaderFolder });
  pixel("video.shader.reload", ["重新载入着色器"], "video-source", { ...gpu, run: reloadShaders });
  for (const { id, label } of TONE_MAPPINGS)
    pixel(`video.hdr.${id}`, ["HDR 色调映射", label], "video-hdr", { radio: true, checked: () => look.render.toneMapping === id, ...gpu, run: () => setTone(id) });
  pixel("video.hdr.peak", ["HDR 色调映射", "动态峰值检测"], "video-hdr-peak", { checked: () => look.render.peakDetect, ...gpu, run: switchPeak });
  pixel("video.hdr.settings", ["HDR 设置…"], "video-hdr", { ...gpu, run: () => showWindow(3) });
  // 403 反交错 (E10, BWDIF only).
  commands.add({
    id: "video.deinterlace.cycle",
    path: ["视频", "反交错", "按序切换"],
    order: 403,
    group: "video-source",
    bindings: ["Ctrl+I"],
    run: () => {
      const index = DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode);
      setDeinterlace(DEINTERLACE_MODES[(index + 1) % DEINTERLACE_MODES.length]!.id);
    },
  });
  for (const { id, label } of DEINTERLACE_MODES)
    commands.add({ id: `video.deinterlace.${id}`, path: ["视频", "反交错", label], order: 403, group: "video-source", radio: true, checked: () => look.deinterlace.mode === id, run: () => setDeinterlace(id) });
  for (const { id, label } of DEINTERLACE_OUTPUTS)
    commands.add({ id: `video.deinterlace.output.${id}`, path: ["视频", "反交错", "输出", label], order: 403, group: "video-deinterlace-output", radio: true, checked: () => look.deinterlace.output === id, run: () => setOutput(id) });
  // 404 比例 (E11): display mode, then the aspect ratios.
  for (const { id, label } of FITS)
    commands.add({
      id: `video.fit.${id}`,
      path: ["视频", "比例", id === "fill" ? "填满窗口（裁去多余）" : label],
      order: 404,
      group: "video-display",
      radio: true,
      checked: () => look.display.fit === id,
      run: () => setFit(id),
    });
  commands.add({ id: "video.aspect.cycle", path: ["视频", "比例", "依次切换比例"], order: 404, group: "video-aspect", bindings: ["Shift+F6"], run: () => setAspect(nextAspect(look.display)) });
  commands.add({ id: "video.aspect.default", path: ["视频", "比例", "默认（按来源）"], order: 404, group: "video-aspect", radio: true, checked: () => look.display.aspect === null, run: () => setAspect(null) });
  RATIOS.forEach(({ label, aspect }, i) =>
    commands.add({ id: `video.aspect.${i}`, path: ["视频", "比例", label], order: 404, group: "video-aspect", radio: true, checked: () => aspectIndex(look.display) === i + 1, run: () => setAspect(aspect) }),
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
    custom.commands.add({
      id: "video.aspect.custom",
      path: ["视频", "比例", text ? `自定义（${text}）` : "自定义（x:y）"],
      order: 404,
      group: "video-aspect",
      radio: true,
      enabled: text !== "",
      reason: text ? null : "先用「自定义比例…」输入",
      checked: () => aspectIndex(look.display) === RATIOS.length + 1,
      run: () => setAspect(parseAspect(text)),
    });
    custom.commands.add({ id: "video.aspect.edit", path: ["视频", "比例", "自定义比例…"], order: 404, group: "video-aspect", run: openCustom });
  };
  addCustom();
  // 405 裁剪/拉伸 (E12).
  commands.add({ id: "video.reframe.none", path: ["视频", "裁剪/拉伸", "不使用"], order: 405, group: "video-display", radio: true, checked: () => look.reframe.mode === "none", run: () => setReframe("none") });
  for (const mode of ["pad", "crop"] as const)
    RATIOS.forEach(({ label }, ratio) =>
      commands.add({
        id: `video.reframe.${mode}.${ratio}`,
        path: ["视频", "裁剪/拉伸", `按 ${label} ${mode === "pad" ? "拉伸" : "裁剪"}`],
        order: 405,
        group: "video-display",
        radio: true,
        checked: () => look.reframe.mode === mode && look.reframe.ratio === ratio,
        run: () => setReframe(mode, ratio),
      }),
    );
  const positions: [string, string, number, number][] = [
    ["reset", "位置复位", 0, 0],
    ["left", "左移", -POSITION_STEP, 0],
    ["right", "右移", POSITION_STEP, 0],
    ["up", "上移", 0, -POSITION_STEP],
    ["down", "下移", 0, POSITION_STEP],
  ];
  for (const [id, title, dx, dy] of positions)
    commands.add({ id: `video.reframe.${id}`, path: ["视频", "裁剪/拉伸", title], order: 405, group: "video-reframe-position", repeat: id !== "reset", run: () => move(dx, dy) });
  // 406 帧位 (E13): keypad keys plus mpv-style Alt keys for keyboards without one.
  const frameCommand = (id: string, title: string, group: string, bindings: string[], run: () => void, repeat = true) =>
    commands.add({ id: `video.frame.${id}`, path: ["视频", "帧位", title], order: 406, group, bindings, repeat, run });
  frameCommand("reset", "帧位复位", "video-display", ["Numpad5", "Alt+Backspace"], () => resetFrame(false), false);
  frameCommand("zoom-in", "全帧 +", "video-display", ["Numpad9", "Alt+Plus"], () => zoomBy(ZOOM_STEP));
  frameCommand("zoom-out", "全帧 -", "video-display", ["Numpad1", "Alt+-"], () => zoomBy(1 / ZOOM_STEP));
  frameCommand("right", "偏移 →", "video-frame-pan", ["Ctrl+Numpad6", "Alt+ArrowRight"], () => panBy(PAN_STEP, 0, "右"));
  frameCommand("left", "偏移 ←", "video-frame-pan", ["Ctrl+Numpad4", "Alt+ArrowLeft"], () => panBy(-PAN_STEP, 0, "左"));
  frameCommand("up", "偏移 ↑", "video-frame-pan", ["Ctrl+Numpad8", "Alt+ArrowUp"], () => panBy(0, -PAN_STEP, "上"));
  frameCommand("down", "偏移 ↓", "video-frame-pan", ["Ctrl+Numpad2", "Alt+ArrowDown"], () => panBy(0, PAN_STEP, "下"));
  frameCommand("centre", "帧位居中", "video-frame-pan", ["Ctrl+Numpad5"], () => resetFrame(true), false);
  // 408 图像处理 (E04–E06, E08; 高斯模糊 moved here, same id and key).
  const processing = (id: string, title: string, group: string, extra: { bindings?: string[]; checked: () => boolean; run: () => void }) =>
    commands.add({ id, path: ["视频", "图像处理", title], order: 408, group, ...extra });
  processing("video.flip.vertical", "上下翻转", "video-effects", { bindings: ["Ctrl+P"], checked: () => look.geometry.flipV, run: () => flip("flipV", "上下翻转") });
  processing("video.flip.horizontal", "左右翻转", "video-effects", { bindings: ["Ctrl+Z"], checked: () => look.geometry.flipH, run: () => flip("flipH", "左右翻转") });
  processing("video.gaussian", "高斯模糊", "video-sharpness", {
    bindings: ["Ctrl+B"],
    checked: () => look.picture.blurEnabled,
    run: () => {
      look.picture.blurEnabled = !look.picture.blurEnabled;
      change();
      osd(`高斯模糊: ${onOff(look.picture.blurEnabled)}`);
    },
  });
  const effect = (id: string, title: string, group: string, toggle: () => Toggle, bindings?: string[]) =>
    processing(id, title, group, { ...(bindings ? { bindings } : {}), checked: () => toggle().on, run: () => switchEffect(toggle(), title) });
  effect("video.sharpen", "锐化", "video-sharpness", () => effects.sharpen, ["Ctrl+R"]);
  effect("video.edge", "边缘锐化", "video-sharpness", () => effects.edge);
  effect("video.deblock", "去块", "video-cleanup", () => effects.deblock, ["Ctrl+H"]);
  effect("video.deband", "消色带", "video-cleanup", () => effects.deband);
  const denoiseKeys: Record<string, string[] | undefined> = { d3: ["Ctrl+N"], temporal: ["Ctrl+M"] };
  for (const mode of DENOISE_MODES)
    effect(`video.denoise.${mode}`, denoiseLabels[mode], "video-cleanup", () => effects.denoise[mode], denoiseKeys[mode]);
  // E18: 倍帧 adds frameBlend once the source rate is known; 运动补偿 is a GPU render option.
  processing("video.frame-blend", "倍帧", "video-frames", { checked: () => effects.frameBlend, run: () => setBlend(!effects.frameBlend, true) });
  commands.add({
    id: "video.interpolation",
    path: ["视频", "图像处理", "运动补偿"],
    order: 408,
    group: "video-frames",
    checked: () => look.render.interpolation,
    ...gpu,
    run: () => setInterpolation(!look.render.interpolation, true),
  });
  // 409 图像旋转 (E09).
  commands.add({
    id: "video.rotate.cycle",
    path: ["视频", "图像旋转", "按序切换"],
    order: 409,
    group: "video-effects",
    bindings: ["Alt+K"],
    run: () => rotate(ROTATIONS[(ROTATIONS.indexOf(look.geometry.rotate) + 1) % ROTATIONS.length]!),
  });
  for (const degrees of ROTATIONS)
    commands.add({
      id: `video.rotate.${degrees}`,
      path: ["视频", "图像旋转", degrees === 0 ? "不旋转" : `${degrees}°`],
      order: 409,
      group: "video-effects",
      radio: true,
      checked: () => look.geometry.rotate === degrees,
      run: () => rotate(degrees),
    });
  // 440–449 the window and the colour keys.
  commands.add({ id: "video.picture", path: ["视频", "画面调节…"], order: 440, group: "video-color", run: () => showWindow() });
  commands.add({ id: "video.reset", path: ["视频", "复位"], order: 441, group: "video-color", bindings: ["Q"], run: reset });
  for (const [label, field, down, up, order] of steps) {
    // All colour steps repeat while held.
    commands.add({ id: `video.${field}.down`, path: ["视频", `${label} -1%`], order, group: "video-color", bindings: [down], repeat: true, run: () => step(label, field, -1) });
    commands.add({ id: `video.${field}.up`, path: ["视频", `${label} +1%`], order: order + 1, group: "video-color", bindings: [up], repeat: true, run: () => step(label, field, 1) });
  }
  apply();
  applyRender();
}

/**
 * The window's status line: one short Chinese line for a failure, the detail in a tooltip; the chain
 * first, then the display layer, then the render options. Software rendering is not a failure here:
 * the GPU controls are greyed with the reason instead.
 */
function statusLine(filter: FilterStatus, view: FilterStatus, render: FilterStatus) {
  if (filter.state === "failed")
    return { failed: true, status: filter.error.code === "unsupported" ? "当前来源没有画面" : "画面处理失败", detail: filter.error.message };
  if (view.state === "failed") return { failed: true, status: "画面显示不可用", detail: viewReason(view.error) };
  if (render.state === "failed" && render.error.code !== "unsupported")
    return {
      failed: true,
      status: shaderFailed(render.error) ? "着色器载入失败" : render.error.code === "conflict" ? "渲染选项不可用" : "渲染设置失败",
      detail: renderReason(render.error),
    };
  return { failed: false, status: "", detail: "" };
}

/** mpv compiles a shader on first use; the player pushes this later (it does not name the file). */
function shaderFailed(error: PluginError) {
  return error.code === "failed" && error.message.startsWith("shader compile failed");
}

/** Why the render options refused; `conflict` means another plugin owns them. */
function renderReason(error: PluginError) {
  return error.code === "conflict" ? "另一个插件正在控制渲染选项" : error.message;
}

/** Why the display layer (比例, 帧位) refused; `conflict` means another plugin owns it. */
function viewReason(error: PluginError) {
  return error.code === "conflict" ? "另一个插件正在控制画面显示" : error.message;
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
