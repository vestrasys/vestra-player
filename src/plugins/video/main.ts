import type { FilterStatus, Plugin } from "../../sdk/plugin/index.ts";
import {
  DEINTERLACE_MODES,
  DEINTERLACE_OUTPUTS,
  DENOISE_MODES,
  POSITION_STEP,
  RATIOS,
  REFRAME_MODES,
  ROTATIONS,
  TABS,
  clamp,
  defaultEffects,
  defaultLook,
  defaults,
  denoiseLabels,
  droppedTooMany,
  nodesFor,
  parse,
  presets,
  reframeLabel,
  serialize,
  upright,
} from "./model.ts";
import type { DeinterlaceMode, DeinterlaceOutput, FrameCount, Look, ReframeMode, Rotation, Toggle } from "./model.ts";

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
const GPU = "需要 GPU 渲染，当前版本不支持";
const LATER = "倍帧需要核心的 frameBlend 节点，后续版本提供";
const signed = (n: number) => (n > 0 ? `+${n}` : `${n}`);
const onOff = (on: boolean) => (on ? "开" : "关");

/**
 * Picture tool: one four-page window, one owned video filter and the 视频 ▶ menu.
 * Colour, effects, deinterlace and crop/pad are global (`settings.json`); flip and
 * rotation belong to the current source and reset when it changes.
 */
export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  const store = settingsStore(plugin);
  const { tab: savedTab, ...saved } = parse(store.value);
  const look: Look = { ...defaultLook(), ...saved };
  let tab = savedTab;
  let enabled = true;

  const filter = plugin.video.filter("picture");
  const window = plugin.windows.create("picture.slint");
  const persist = () => store.save(serialize(look, tab));
  let sent = "";
  /** The SDK keeps one change in flight; an unchanged chain is not resent unless it failed. */
  const apply = () => {
    const nodes = enabled ? nodesFor(look) : [];
    const key = JSON.stringify(nodes);
    if (key === sent && filter.status.state !== "failed") return;
    sent = key;
    filter.set(nodes);
  };
  const change = () => {
    apply();
    persist();
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
      flipV: look.geometry.flipV,
      flipH: look.geometry.flipH,
      rotateIndex: ROTATIONS.indexOf(look.geometry.rotate),
      reframeIndex: REFRAME_MODES.indexOf(look.reframe.mode),
      ratioIndex: look.reframe.ratio,
      deinterlaceIndex: DEINTERLACE_MODES.findIndex((m) => m.id === look.deinterlace.mode),
      outputIndex: DEINTERLACE_OUTPUTS.findIndex((m) => m.id === look.deinterlace.output),
      tabIndex: tab,
      effectEnabled: enabled,
      failed: filter.status.state === "failed",
      status: describe(filter.status),
      detail: filter.status.state === "failed" ? filter.status.error.message : "",
    };
  });
  /** Store a slider value; true when the field is known. */
  const storeValue = (field: string, value: number) => {
    if (!Number.isFinite(value)) return null;
    if (field === "blurStrength") look.picture.blurStrength = clamp(value, 0, 10);
    else if ((colors as string[]).includes(field)) look.picture[field as Color] = clamp(value, -100, 100);
    else if ((strengths as string[]).includes(field)) toggleOf(field as Strength).strength = clamp(Math.round(value), 0, 100);
    else return null;
    return (strengths as string[]).includes(field) ? "rebuild" : "live";
  };
  // eq, hue and gblur change in place while playing; the other filters rebuild, so they wait for release.
  window.on("adjust", (field: string, value: number) => {
    if (storeValue(field, value) === "live") change();
  });
  window.on("commit", (field: string, value: number) => {
    if (storeValue(field, value)) change();
  });
  window.on("resetField", (field: string) => {
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
  });
  window.on("toggle", (field: string, on: boolean) => {
    const value = on === true;
    if (field === "blur") look.picture.blurEnabled = value;
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
    else return;
    change();
  });
  window.on("tabChanged", (index: number) => {
    if (!Number.isInteger(index) || index < 0 || index >= TABS || index === tab) return;
    tab = index;
    persist();
  });
  window.on("retry", () => filter.retry());

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
  });

  // E10: field output doubles the frame rate; warn once per source when it drops many frames.
  let counted: (FrameCount & { source: number }) | null = null;
  let warned: number | null = null;
  const monitor = setInterval(async () => {
    const state = player.state;
    const watching = enabled && look.deinterlace.mode !== "off" && look.deinterlace.output === "field";
    if (!watching || state.phase !== "ready" || state.paused || warned === state.sourceId) {
      counted = null;
      return;
    }
    const stats = await player.stats().catch(() => null);
    if (!stats || stats.video === null || stats.framesDropped === null) return;
    const now = { source: state.sourceId, shown: stats.framesShown, dropped: stats.framesDropped };
    if (counted && counted.source === now.source && droppedTooMany(counted, now)) {
      warned = now.source;
      osd("反交错按场输出丢帧较多，可改为按帧");
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
  commands.add({ id: "video.frame-blend", path: ["视频", "图像处理", "倍帧"], order: 408, group: "video-frames", enabled: false, reason: LATER, run: () => undefined });
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

/** One short Chinese line; only a failure needs words (the detail goes in a tooltip). */
function describe(status: FilterStatus) {
  if (status.state !== "failed") return "";
  return status.error.code === "unsupported" ? "当前来源没有画面" : "画面处理失败";
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
