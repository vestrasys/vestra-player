import type { FilterStatus, Plugin, PluginError } from "../../sdk/plugin/index.ts";
import {
  DEINTERLACE_MODES,
  FITS,
  PAN_STEP,
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
  nextAspect,
  nodesFor,
  parse,
  parseAspect,
  presets,
  reframeLabel,
  serialize,
  upright,
} from "./model.ts";
import type { DeinterlaceMode, DeinterlaceOutput, Fit, Frame, FrameCount, Look, ReframeMode, Rotation, Toggle } from "./model.ts";

type Color = "brightness" | "contrast" | "saturation" | "hue" | "gamma";
const colors: readonly Color[] = ["brightness", "contrast", "saturation", "hue", "gamma"];
/** Keyboard steps (reference player W/E, R/T, Y/U; 色调 I/O): label, field, keys for −1 and +1, menu order. */
const steps: readonly [string, Color, string, string, number][] = [
  ["亮度", "brightness", "W", "E", 442],
  ["对比度", "contrast", "R", "T", 444],
  ["饱和度", "saturation", "Y", "U", 446],
  ["色调", "hue", "I", "O", 448],
];
/** Effect rows with one 0–100 strength; their filters rebuild, so a drag commits on release. */
type Strength = "sharpen" | "edge" | "denoise" | "deblock" | "deband";
const strengths: readonly Strength[] = ["sharpen", "edge", "denoise", "deblock", "deband"];
/** E16, E17 and 运动补偿 wait for the GPU rendering prototype (round 4a §7.2). */
const GPU = "等待 GPU 渲染原型结论";
/** E18: the source rate is read this long after the chain without frameBlend is in place, this many times at most. */
const MEASURE_DELAY_MS = 400;
const MEASURE_TRIES = 8;
const ASPECT_FORMAT = "格式：16:9 或 2.39";
const percent = (n: number) => `${Math.round(n * 100)}%`;
const signed = (n: number) => (n > 0 ? `+${n}` : `${n}`);
const onOff = (on: boolean) => (on ? "开" : "关");

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
  const persist = () => store.save(serialize(look, tab));

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
      tabIndex: tab,
      effectEnabled: enabled,
      ...statusLine(filter.status, view.status),
    };
  });
  /** Store a slider value; how it takes effect (null when the field is unknown). */
  const storeValue = (field: string, value: number) => {
    if (!Number.isFinite(value)) return null;
    if (field === "blurStrength") look.picture.blurStrength = clamp(value, 0, 10);
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = clamp(value, -100, 100);
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).strength = clamp(Math.round(value), 0, 100);
    else if (field === "zoom") frame.zoom = zoomOf(2 ** clamp(value, -2, 2));
    else return null;
    return field === "zoom" ? "view" : (strengths as string[]).includes(field) ? "rebuild" : "live";
  };
  // eq, hue and gblur change in place while playing; the other filters rebuild, so they wait for release.
  // Zoom is a display property: it follows the drag.
  window.on("adjust", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "view") showView();
    else if (kind === "live") change();
  });
  window.on("commit", (field: string, value: number) => {
    const kind = storeValue(field, value);
    if (kind === "view") showView();
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
    else if (field === "frameBlend") switchBlend(value);
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
    } else return;
    change();
  });
  window.on("customAspect", () => openCustom());
  window.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    persist();
  });
  window.on("retry", () => {
    if (filter.status.state === "failed") filter.retry();
    if (view.status.state === "failed") showView();
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
  // A new source (or one becoming ready) needs its own rate before 倍帧 can add the node.
  player.onChange(() => {
    if (blendRate() !== sentBlend) apply();
    if (wantsMeasure()) void measure();
  });

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

  // ---------------------------------------------------------------- menu 视频 ▶
  const { commands } = plugin;
  // 402 像素着色 (E16, E17): GPU rendering first; shown disabled with the reason.
  commands.add({ id: "video.shader.use", path: ["视频", "像素着色", "使用着色器"], order: 402, group: "video-source", bindings: ["Shift+P"], enabled: false, reason: GPU, run: () => undefined });
  commands.add({ id: "video.shader.hdr", path: ["视频", "像素着色", "HDR 色调映射"], order: 402, group: "video-source", enabled: false, reason: GPU, run: () => undefined });
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
  commands.add({ id: "video.aspect.cycle", path: ["视频", "比例", "依次切换比例"], order: 404, group: "video-aspect", bindings: ["Ctrl+F6"], run: () => setAspect(nextAspect(look.display)) });
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
  // 406 帧位 (E13): reference player's keypad keys plus mpv-style Alt keys for keyboards without one.
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
  // E18: 倍帧 adds frameBlend once the source rate is known; 运动补偿 waits for GPU rendering, so
  // their mutual exclusion never triggers yet.
  processing("video.frame-blend", "倍帧", "video-frames", {
    checked: () => effects.frameBlend,
    run: () => {
      switchBlend(!effects.frameBlend);
      change();
      osd(`倍帧: ${onOff(effects.frameBlend)}${effects.frameBlend ? offNote() : ""}`);
    },
  });
  commands.add({ id: "video.interpolation", path: ["视频", "图像处理", "运动补偿"], order: 408, group: "video-frames", enabled: false, reason: GPU, run: () => undefined });
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
  // 440–449 the window and reference player's colour keys.
  commands.add({ id: "video.picture", path: ["视频", "画面调节…"], order: 440, group: "video-color", run: () => window.show() });
  commands.add({ id: "video.reset", path: ["视频", "复位"], order: 441, group: "video-color", bindings: ["Q"], run: reset });
  for (const [label, field, down, up, order] of steps) {
    // All colour steps repeat while held.
    commands.add({ id: `video.${field}.down`, path: ["视频", `${label} -1%`], order, group: "video-color", bindings: [down], repeat: true, run: () => step(label, field, -1) });
    commands.add({ id: `video.${field}.up`, path: ["视频", `${label} +1%`], order: order + 1, group: "video-color", bindings: [up], repeat: true, run: () => step(label, field, 1) });
  }
  apply();
}

/** The window's status line: one short Chinese line for a failure, the detail in a tooltip; the chain first. */
function statusLine(filter: FilterStatus, view: FilterStatus) {
  if (filter.state === "failed")
    return { failed: true, status: filter.error.code === "unsupported" ? "当前来源没有画面" : "画面处理失败", detail: filter.error.message };
  if (view.state === "failed") return { failed: true, status: "画面显示不可用", detail: viewReason(view.error) };
  return { failed: false, status: "", detail: "" };
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
