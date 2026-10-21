import assert from "node:assert/strict";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { aspectIndex, aspectLabel, blendFps, defaultLook, defaults, droppedTooMany, isHdr, isShaderName, nextAspect, nodesFor, parse, parseAspect, peakOf, serialize, shaderList } from "../model.ts";
import type { Look, Picture } from "../model.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Node = { kind: string } & Record<string, any>;
const latest = (host: TestHost) => host.sent("processing.replace").at(-1)!;
const chain = (host: TestHost) => latest(host).nodes as Node[];
const kinds = (host: TestHost) => chain(host).map((n) => n.kind);
const node = (host: TestHost, kind: string) => chain(host).find((n) => n.kind === kind);
const eq = (host: TestHost) => node(host, "eq")!;
const look = (picture: Partial<Picture> = {}, more: Partial<Look> = {}): Look => ({
  ...defaultLook(),
  ...more,
  picture: { ...defaults, ...picture },
});
const neutralEq = { kind: "eq", brightness: 0, contrast: 1, saturation: 1, gamma: 1 };
const autoDeinterlace = { kind: "deinterlace", when: "interlaced", output: "frame" };
async function playing(dataDirectory?: string) {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const started = await start(host, dataDirectory);
  await host.command("video.picture").run();
  return { ...started, host, window: host.window("picture") };
}
/** `player.stats()` with a picture at `fps` (`undefined` = no picture). */
function setFps(host: TestHost, fps: number | null | undefined) {
  host.stats = {
    ...host.stats,
    video:
      fps === undefined
        ? null
        : { codec: "h264", width: 1920, height: 1080, fps, decoder: "h264", pixelFormat: "yuv420p", colorspace: "bt.709", displayWidth: 1920, displayHeight: 1080, displayFps: null, bitrate: null },
  };
}
/** Past one source-rate read (400 ms after the chain without frameBlend is in place). */
const measured = async (host: TestHost) => {
  await new Promise((resolve) => setTimeout(resolve, 500));
  await host.settle();
};
async function withData(run: (data: string) => Promise<void>) {
  const data = await mkdtemp(join(tmpdir(), "vestra-video-"));
  try {
    await run(data);
  } finally {
    await rm(data, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------- model

test("neutral: automatic deinterlace and one eq node; blur, hue and strengths add nodes only when they act", () => {
  assert.deepEqual(nodesFor(defaultLook()), [autoDeinterlace, neutralEq]);
  assert.deepEqual(nodesFor(look({ blurEnabled: true, blurStrength: 4 })).at(-1), { kind: "gblur", sigma: 4, steps: 3 });
  assert.equal(nodesFor(look({ blurEnabled: true, blurStrength: 0 })).length, 2);
  assert.deepEqual(nodesFor(look({ blurEnabled: true, blurStrength: 15 })).at(-1), { kind: "gblur", sigma: 10, steps: 3 });
  assert.deepEqual(nodesFor(look({ hue: 100 })).at(-1), { kind: "hue", degrees: 180 }, "E02: 1% = 1.8°");
  assert.deepEqual(nodesFor(look({ hue: -6 })).at(-1), { kind: "hue", degrees: -10.8 });
  const zero = defaultLook();
  zero.effects.sharpen = { on: true, strength: 0 };
  zero.effects.deband = { on: true, strength: 0 };
  assert.deepEqual(nodesFor(zero), [autoDeinterlace, neutralEq], "on at strength 0 makes no node");
  assert.deepEqual(nodesFor({ ...defaultLook(), deinterlace: { mode: "off", output: "frame" } }), [neutralEq]);
});

test("the full chain keeps the fixed order and maps strengths to the host's ranges", () => {
  const all = look(
    { brightness: 10, hue: 20, blurEnabled: true, blurStrength: 2 },
    { deinterlace: { mode: "on", output: "field" }, reframe: { mode: "crop", ratio: 4, x: 50, y: 0 }, geometry: { flipH: false, flipV: true, rotate: 180 } },
  );
  const e = all.effects;
  e.sharpen = { on: true, strength: 100 };
  e.edge = { on: true, strength: 50 };
  e.denoise.d3 = { on: true, strength: 40 };
  e.denoise.gradual = { on: true, strength: 100 };
  e.denoise.temporal = { on: true, strength: 100 };
  e.deblock = { on: true, strength: 50 };
  e.deband = { on: true, strength: 100 };
  const nodes = nodesFor(all);
  assert.deepEqual(
    nodes.map((n) => n.kind),
    ["deinterlace", "reframe", "transform", "denoise", "denoise", "denoise", "deblock", "deband", "eq", "hue", "sharpen", "sharpen", "gblur"],
  );
  assert.deepEqual(nodes[0], { kind: "deinterlace", when: "all", output: "field" });
  assert.deepEqual(nodes[1], { kind: "reframe", mode: "crop", aspect: 2.35, x: 0, y: -1 });
  assert.deepEqual(nodes[2], { kind: "transform", rotate: 180, flipH: false, flipV: true });
  assert.deepEqual(nodes[3], { kind: "denoise", lumaSpatial: 4, chromaSpatial: 3, lumaTemporal: 6, chromaTemporal: 4.5 }, "3D at 40 = hqdn3d defaults");
  assert.deepEqual(nodes[4], { kind: "denoise", lumaSpatial: 6, chromaSpatial: 4.5, lumaTemporal: 6, chromaTemporal: 4.5 }, "渐进: balanced");
  assert.deepEqual(nodes[5], { kind: "denoise", lumaSpatial: 2, chromaSpatial: 1.5, lumaTemporal: 20, chromaTemporal: 15 }, "瞬态: temporal");
  assert.deepEqual(nodes[6], { kind: "deblock", strength: "strong" });
  assert.deepEqual(nodes[7], { kind: "deband", threshold: 0.05, range: 16 });
  assert.deepEqual(nodes[10], { kind: "sharpen", amount: 1.5, size: "small" });
  assert.deepEqual(nodes[11], { kind: "sharpen", amount: 0.5, size: "large" });
  e.deblock.strength = 49;
  assert.deepEqual(nodesFor(all)[6], { kind: "deblock", strength: "weak" });
  e.deband.strength = 0.5;
  assert.ok((nodesFor(all)[7] as { threshold: number }).threshold >= 0.003);
});

test("flip happens before rotation: a quarter turn moves the flip to the other axis", () => {
  const geometry = (flipH: boolean, flipV: boolean, rotate: 0 | 90 | 180 | 270) =>
    nodesFor({ ...defaultLook(), geometry: { flipH, flipV, rotate } }).find((n) => n.kind === "transform");
  assert.equal(geometry(false, false, 0), undefined);
  assert.deepEqual(geometry(true, false, 0), { kind: "transform", rotate: 0, flipH: true, flipV: false });
  assert.deepEqual(geometry(true, false, 90), { kind: "transform", rotate: 90, flipH: false, flipV: true });
  assert.deepEqual(geometry(false, true, 270), { kind: "transform", rotate: 270, flipH: true, flipV: false });
  assert.deepEqual(geometry(true, false, 180), { kind: "transform", rotate: 180, flipH: true, flipV: false });
});

test("settings parse leniently; an unknown version starts from the defaults", () => {
  const base = defaultLook();
  assert.deepEqual(parse(null), { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe, display: base.display, render: base.render, tab: 0 });
  const parsed = parse({
    version: 1,
    picture: { brightness: 500, hue: 12.6, blurEnabled: true, blurStrength: "x" },
    effects: { sharpen: { on: true, strength: 70 }, denoise: { temporal: { on: true, strength: 300 }, shown: "temporal" }, deblock: 4 },
    deinterlace: { mode: "off", output: "sideways" },
    reframe: { mode: "pad", ratio: 9, x: 120, y: 10 },
    tab: 3,
  });
  assert.equal(parsed.picture.brightness, 100);
  assert.equal(parsed.picture.hue, 13);
  assert.equal(parsed.picture.blurStrength, defaults.blurStrength);
  assert.deepEqual(parsed.effects.sharpen, { on: true, strength: 70 });
  assert.deepEqual(parsed.effects.denoise.temporal, { on: true, strength: 100 });
  assert.equal(parsed.effects.denoise.shown, "temporal");
  assert.deepEqual(parsed.effects.deblock, base.effects.deblock);
  assert.deepEqual(parsed.deinterlace, { mode: "off", output: "frame" });
  assert.deepEqual(parsed.reframe, { mode: "pad", ratio: 4, x: 100, y: 10 });
  assert.equal(parsed.tab, 3);
  assert.equal(parse({ version: 2, picture: { brightness: 40 }, tab: 2 }).picture.brightness, 0);
  const written = serialize({ ...base, geometry: { flipH: true, flipV: true, rotate: 90 } }, 2);
  assert.equal(written.version, 1);
  assert.equal("geometry" in written, false, "flip and rotation are per source");
  const { tab, ...again } = parse(JSON.parse(JSON.stringify(written)));
  assert.equal(tab, 2);
  assert.deepEqual(again, { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe, display: base.display, render: base.render });
});

test("field-output frame drops: more than 5 % of the frames since the last count", () => {
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 200, dropped: 4 }), false);
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 190, dropped: 10 }), true);
  assert.equal(droppedTooMany({ shown: 100, dropped: 5 }, { shown: 100, dropped: 5 }), false);
});

// ---------------------------------------------------------------- menu

test("menu under 视频 follows the chapter's allocation; GPU items are enabled under GPU rendering", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  const declared = [...host.commands.values()].map(({ declaration: d }) => [d.path.slice(1).join(" ▶ "), d.order, d.group, d.bindings.join(",")]);
  const ratios = ["4:3", "16:9", "16:10", "1.85:1", "2.35:1"];
  assert.deepEqual(declared, [
    ["像素着色 ▶ 使用着色器", 402, "video-source", "Shift+P"],
    ["像素着色 ▶ 着色器列表…", 402, "video-source", ""],
    ["像素着色 ▶ 打开着色器文件夹", 402, "video-source", ""],
    ["像素着色 ▶ 重新载入着色器", 402, "video-source", ""],
    ...["自动", "关", "BT.2390", "Hable", "Mobius", "Reinhard", "裁剪"].map((t) => [`像素着色 ▶ HDR 色调映射 ▶ ${t}`, 402, "video-hdr", ""]),
    ["像素着色 ▶ HDR 色调映射 ▶ 动态峰值检测", 402, "video-hdr-peak", ""],
    ["像素着色 ▶ HDR 设置…", 402, "video-hdr", ""],
    ["反交错 ▶ 按序切换", 403, "video-source", "Ctrl+I"],
    ["反交错 ▶ 自动", 403, "video-source", ""],
    ["反交错 ▶ 总是使用", 403, "video-source", ""],
    ["反交错 ▶ 不使用", 403, "video-source", ""],
    ["反交错 ▶ 输出 ▶ 按帧", 403, "video-deinterlace-output", ""],
    ["反交错 ▶ 输出 ▶ 按场（倍帧率）", 403, "video-deinterlace-output", ""],
    ["比例 ▶ 保持宽高比", 404, "video-display", ""],
    ["比例 ▶ 不保持宽高比", 404, "video-display", ""],
    ["比例 ▶ 填满窗口（裁去多余）", 404, "video-display", ""],
    ["比例 ▶ 依次切换比例", 404, "video-aspect", "Shift+F6"],
    ["比例 ▶ 默认（按来源）", 404, "video-aspect", ""],
    ...ratios.map((r) => [`比例 ▶ ${r}`, 404, "video-aspect", ""]),
    ["比例 ▶ 自定义（x:y）", 404, "video-aspect", ""],
    ["比例 ▶ 自定义比例…", 404, "video-aspect", ""],
    ["裁剪/拉伸 ▶ 不使用", 405, "video-display", ""],
    ...ratios.map((r) => [`裁剪/拉伸 ▶ 按 ${r} 拉伸`, 405, "video-display", ""]),
    ...ratios.map((r) => [`裁剪/拉伸 ▶ 按 ${r} 裁剪`, 405, "video-display", ""]),
    ...["位置复位", "左移", "右移", "上移", "下移"].map((t) => [`裁剪/拉伸 ▶ ${t}`, 405, "video-reframe-position", ""]),
    ["帧位 ▶ 帧位复位", 406, "video-display", "Numpad5,Alt+Backspace"],
    ["帧位 ▶ 全帧 +", 406, "video-display", "Numpad9,Alt+Plus"],
    ["帧位 ▶ 全帧 -", 406, "video-display", "Numpad1,Alt+-"],
    ["帧位 ▶ 偏移 →", 406, "video-frame-pan", "Ctrl+Numpad6,Alt+ArrowRight"],
    ["帧位 ▶ 偏移 ←", 406, "video-frame-pan", "Ctrl+Numpad4,Alt+ArrowLeft"],
    ["帧位 ▶ 偏移 ↑", 406, "video-frame-pan", "Ctrl+Numpad8,Alt+ArrowUp"],
    ["帧位 ▶ 偏移 ↓", 406, "video-frame-pan", "Ctrl+Numpad2,Alt+ArrowDown"],
    ["帧位 ▶ 帧位居中", 406, "video-frame-pan", "Ctrl+Numpad5"],
    ["图像处理 ▶ 上下翻转", 408, "video-effects", "Ctrl+P"],
    ["图像处理 ▶ 左右翻转", 408, "video-effects", "Ctrl+Z"],
    ["图像处理 ▶ 高斯模糊", 408, "video-sharpness", "Ctrl+B"],
    ["图像处理 ▶ 锐化", 408, "video-sharpness", "Ctrl+R"],
    ["图像处理 ▶ 边缘锐化", 408, "video-sharpness", ""],
    ["图像处理 ▶ 去块", 408, "video-cleanup", "Ctrl+H"],
    ["图像处理 ▶ 消色带", 408, "video-cleanup", ""],
    ["图像处理 ▶ 3D 降噪", 408, "video-cleanup", "Ctrl+N"],
    ["图像处理 ▶ 渐进降噪", 408, "video-cleanup", ""],
    ["图像处理 ▶ 瞬态降噪", 408, "video-cleanup", "Ctrl+M"],
    ["图像处理 ▶ 倍帧", 408, "video-frames", ""],
    ["图像处理 ▶ 运动补偿", 408, "video-frames", ""],
    ["图像旋转 ▶ 按序切换", 409, "video-effects", "Alt+K"],
    ["图像旋转 ▶ 不旋转", 409, "video-effects", ""],
    ["图像旋转 ▶ 90°", 409, "video-effects", ""],
    ["图像旋转 ▶ 180°", 409, "video-effects", ""],
    ["图像旋转 ▶ 270°", 409, "video-effects", ""],
    ["画面调节…", 440, "video-color", ""],
    ["复位", 441, "video-color", "Q"],
    ["亮度 -1%", 442, "video-color", "W"],
    ["亮度 +1%", 443, "video-color", "E"],
    ["对比度 -1%", 444, "video-color", "R"],
    ["对比度 +1%", 445, "video-color", "T"],
    ["饱和度 -1%", 446, "video-color", "Y"],
    ["饱和度 +1%", 447, "video-color", "U"],
    ["色调 -1%", 448, "video-color", "I"],
    ["色调 +1%", 449, "video-color", "O"],
  ]);
  assert.deepEqual(host.conflicts, []);
  await host.settle();
  for (const id of ["video.shader.use", "video.interpolation"])
    assert.deepEqual(host.command(id).state, { enabled: true, checked: false, busy: false, reason: null });
  assert.equal(host.command("video.hdr.auto").state.checked, true);
  assert.equal(host.command("video.hdr.peak").state.checked, true);
  assert.deepEqual(host.command("video.frame-blend").state, { enabled: true, checked: false, busy: false, reason: null });
  assert.equal(host.command("video.aspect.custom").state.enabled, false, "自定义 waits for a ratio");
  assert.equal(host.command("video.fit.keep").state.checked, true);
  assert.equal(host.command("video.aspect.default").state.checked, true);
  assert.equal(host.command("video.deinterlace.auto").state.checked, true);
  assert.equal(host.command("video.deinterlace.output.frame").state.checked, true);
  assert.equal(host.command("video.reframe.none").state.checked, true);
  assert.equal(host.command("video.rotate.0").state.checked, true);
  await host.command("video.picture").run();
  await host.command("video.picture").run();
  assert.equal(host.sent("ui.open").length, 1, "the window opens once");
  await stop();
});

// ---------------------------------------------------------------- colour

test("PotPlayer keys step colours by 1% with on-screen feedback; Q resets; Ctrl+B toggles blur", async () => {
  const { host, window, stop } = await playing();
  for (const key of ["E", "E", "W", "T", "Y"]) await host.press(key);
  await host.settle();
  assert.deepEqual(host.osd, ["亮度 +1% (+1%)", "亮度 +1% (+2%)", "亮度 -1% (+1%)", "对比度 +1% (+1%)", "饱和度 -1% (-1%)"]);
  assert.equal(eq(host).brightness, 0.01);
  assert.equal(eq(host).contrast, 2 ** 0.01);
  assert.equal(eq(host).saturation, 0.99);
  await host.press("R");
  await host.press("U");
  assert.equal(host.osd.at(-1), "饱和度 +1% (0%)");
  // A dragged fractional value steps from its rounded value; limits hold.
  window.emit("adjust", "brightness", 99.6);
  await host.settle();
  await host.press("E");
  await host.press("E");
  assert.equal(window.props.brightness, 100);
  assert.equal(host.osd.at(-1), "亮度 +1% (+100%)");
  await host.press("Ctrl+B");
  await host.settle();
  assert.equal(host.osd.at(-1), "高斯模糊: 开");
  assert.equal(host.command("video.gaussian").state.checked, true);
  assert.equal(window.props["blur-enabled"], true);
  assert.equal(kinds(host).at(-1), "gblur");
  await host.press("Ctrl+B");
  assert.equal(host.osd.at(-1), "高斯模糊: 关");
  await host.press("Q");
  await host.settle();
  assert.equal(host.osd.at(-1), "画面调节已复位");
  assert.deepEqual(chain(host), nodesFor(defaultLook()));
  assert.equal(window.props.brightness, 0);
  await stop();
});

test("E02: I / O step the hue, the window follows, Q and 原始 reset it; other presets keep it at 0", async () => {
  const { host, window, stop } = await playing();
  await host.press("O");
  await host.press("O");
  await host.press("I");
  await host.settle();
  assert.deepEqual(host.osd, ["色调 +1% (+1%)", "色调 +1% (+2%)", "色调 -1% (+1%)"]);
  assert.equal(window.props.hue, 1);
  assert.deepEqual(node(host, "hue"), { kind: "hue", degrees: 1.8 });
  window.emit("adjust", "hue", 100);
  await host.settle();
  await host.press("O");
  assert.equal(host.osd.at(-1), "色调 +1% (+100%)", "the edge keeps the value and still reports it");
  assert.deepEqual(node(host, "hue"), { kind: "hue", degrees: 180 });
  window.emit("preset", "vivid");
  await host.settle();
  assert.equal(window.props.hue, 0);
  assert.equal(node(host, "hue"), undefined);
  window.emit("adjust", "hue", 30);
  await host.press("Q");
  await host.settle();
  assert.equal(window.props.hue, 0);
  assert.equal(node(host, "hue"), undefined);
  // With 启用画面效果 off the keys still change the value, and say so.
  window.emit("toggle-effect", false);
  await host.press("O");
  await host.settle();
  assert.equal(host.osd.at(-1), "色调 +1% (+1%)（画面效果已关闭）");
  assert.deepEqual(chain(host), []);
  assert.equal(latest(host).resourceId, "picture");
  window.emit("toggle-effect", true);
  await host.settle();
  assert.deepEqual(node(host, "hue"), { kind: "hue", degrees: 1.8 });
  await stop();
});

test("each 默认 button resets only its own row; window edits give no on-screen text", async () => {
  const { host, window, stop } = await playing();
  window.emit("adjust", "contrast", 30);
  window.emit("adjust", "gamma", 20);
  window.emit("adjust", "blurStrength", 9);
  window.emit("commit", "sharpen", 80);
  window.emit("reset-field", "contrast");
  window.emit("reset-field", "blurStrength");
  window.emit("reset-field", "sharpen");
  window.emit("reset-field", "nonsense");
  await host.settle();
  assert.equal(window.props.contrast, 0);
  assert.equal(window.props.gamma, 20);
  assert.equal(window.props["blur-strength"], defaults.blurStrength);
  assert.equal(window.props["sharpen-strength"], 50);
  assert.equal(eq(host).gamma, 2 ** 0.2);
  assert.deepEqual(host.osd, []);
  await stop();
});

test("presets replace colour and blur", async () => {
  const { host, window, stop } = await playing();
  window.emit("preset", "soft");
  await host.settle();
  assert.equal(window.props.contrast, -8);
  assert.equal(window.props["blur-enabled"], true);
  assert.equal(kinds(host).at(-1), "gblur");
  await host.command("video.reset").run();
  await host.settle();
  assert.equal(window.props.contrast, 0);
  assert.deepEqual(chain(host), nodesFor(defaultLook()));
  await stop();
});

// ---------------------------------------------------------------- effects

test("E04–E06: menu keys switch effects at the remembered strength; menu, window and OSD agree", async () => {
  const { host, window, stop } = await playing();
  await host.press("Ctrl+R");
  await host.press("Ctrl+N");
  await host.press("Ctrl+M");
  await host.press("Ctrl+H");
  await host.command("video.edge").run();
  await host.command("video.deband").run();
  await host.command("video.denoise.gradual").run();
  await host.settle();
  assert.deepEqual(host.osd, ["锐化: 开", "3D 降噪: 开", "瞬态降噪: 开", "去块: 开", "边缘锐化: 开", "消色带: 开", "渐进降噪: 开"]);
  assert.deepEqual(kinds(host), ["deinterlace", "denoise", "denoise", "denoise", "deblock", "deband", "eq", "sharpen", "sharpen"]);
  assert.deepEqual(node(host, "deblock"), { kind: "deblock", strength: "weak" });
  assert.equal(host.command("video.sharpen").state.checked, true);
  assert.equal(window.props["sharpen-enabled"], true);
  assert.equal(window.props["denoise-enabled"], true, "the shown mode (3D) is on");
  // The window's 降噪 row shows one mode at a time.
  window.emit("toggle", "denoise", false);
  await host.settle();
  assert.equal(host.command("video.denoise.d3").state.checked, false);
  window.emit("choose", "denoiseMode", 2);
  await host.settle();
  assert.equal(window.props["denoise-index"], 2);
  assert.equal(window.props["denoise-enabled"], true, "瞬态 is still on");
  window.emit("commit", "denoise", 100);
  await host.settle();
  assert.equal(chain(host).filter((n) => n.kind === "denoise").at(-1)!.lumaTemporal, 20);
  await host.press("Ctrl+R");
  assert.equal(host.osd.at(-1), "锐化: 关");
  assert.equal(window.props["sharpen-strength"], 50, "off keeps the strength");
  await stop();
});

test("rebuilding filters commit on release; eq, hue and blur follow the drag", async () => {
  const { host, window, stop } = await playing();
  window.emit("toggle", "sharpen", true);
  window.emit("toggle", "deblock", true);
  await host.settle();
  const before = host.sent("processing.replace").length;
  for (let v = 51; v <= 90; v++) window.emit("adjust", "sharpen", v);
  window.emit("adjust", "deblock", 30);
  await host.settle();
  assert.equal(host.sent("processing.replace").length, before, "no rebuild while dragging");
  assert.equal(window.props["sharpen-strength"], 90, "the slider keeps its value");
  window.emit("commit", "sharpen", 90);
  await host.settle();
  assert.equal(host.sent("processing.replace").length, before + 1);
  assert.deepEqual(node(host, "sharpen"), { kind: "sharpen", amount: 1.35, size: "small" });
  window.emit("commit", "deblock", 40);
  await host.settle();
  assert.equal(host.sent("processing.replace").length, before + 1, "the same 弱 tier sends nothing");
  window.emit("commit", "deblock", 60);
  await host.settle();
  assert.deepEqual(node(host, "deblock"), { kind: "deblock", strength: "strong" });
  window.emit("adjust", "hue", 10);
  await host.settle();
  assert.deepEqual(node(host, "hue"), { kind: "hue", degrees: 18 });
  await stop();
});

test("a burst of slider edits keeps one change in flight and ends on the last value", async () => {
  const { host, window, stop } = await playing();
  const sent = host.sent("processing.replace").length;
  for (let value = 1; value <= 30; value++) window.emit("adjust", "brightness", value);
  await host.settle();
  assert.ok(host.sent("processing.replace").length - sent <= 3);
  assert.equal(eq(host).brightness, 0.3);
  await stop();
});

test("blur strength above the host's 0–10 sigma range is clamped to 10", async () => {
  const { host, window, stop } = await playing();
  window.emit("toggle", "blur", true);
  window.emit("adjust", "blurStrength", 18);
  await host.settle();
  assert.equal(window.props["blur-strength"], 10);
  assert.deepEqual(node(host, "gblur"), { kind: "gblur", sigma: 10, steps: 3 });
  assert.equal(window.props.failed, false);
  await stop();
});

// ---------------------------------------------------------------- picture

test("E08 / E09: flips and rotation with OSD; the window follows; a new source starts upright", async () => {
  const { host, window, stop } = await playing();
  await host.press("Ctrl+P");
  await host.settle();
  assert.equal(host.osd.at(-1), "上下翻转: 开");
  assert.deepEqual(node(host, "transform"), { kind: "transform", rotate: 0, flipH: false, flipV: true });
  assert.equal(window.props["flip-v"], true);
  assert.equal(host.command("video.flip.vertical").state.checked, true);
  for (let i = 0; i < 4; i++) await host.press("Alt+K");
  assert.deepEqual(host.osd.slice(-4), ["图像旋转: 90°", "图像旋转: 180°", "图像旋转: 270°", "图像旋转: 不旋转"]);
  await host.command("video.rotate.90").run();
  await host.settle();
  assert.equal(window.props["rotate-index"], 1);
  assert.equal(host.command("video.rotate.90").state.checked, true);
  assert.deepEqual(node(host, "transform"), { kind: "transform", rotate: 90, flipH: true, flipV: false }, "flipped before turning");
  window.emit("toggle", "flipH", true);
  window.emit("choose", "rotate", 2);
  await host.settle();
  assert.deepEqual(node(host, "transform"), { kind: "transform", rotate: 180, flipH: true, flipV: true });
  const osd = host.osd.length;
  host.openElsewhere("/next.mp4");
  await host.settle();
  assert.equal(node(host, "transform"), undefined);
  assert.equal(window.props["flip-v"], false);
  assert.equal(window.props["rotate-index"], 0);
  assert.equal(host.osd.length, osd, "the reset is silent");
  await stop();
});

test("E10: Ctrl+I cycles 自动 → 总是使用 → 不使用; output by frame or field", async () => {
  const { host, window, stop } = await playing();
  assert.deepEqual(node(host, "deinterlace"), autoDeinterlace);
  await host.press("Ctrl+I");
  await host.settle();
  assert.equal(host.osd.at(-1), "反交错: 总是使用");
  assert.deepEqual(node(host, "deinterlace"), { kind: "deinterlace", when: "all", output: "frame" });
  assert.equal(host.command("video.deinterlace.on").state.checked, true);
  await host.press("Ctrl+I");
  await host.settle();
  assert.equal(host.osd.at(-1), "反交错: 不使用");
  assert.equal(node(host, "deinterlace"), undefined);
  assert.equal(window.props["deinterlace-index"], 2);
  await host.press("Ctrl+I");
  assert.equal(host.osd.at(-1), "反交错: 自动");
  await host.command("video.deinterlace.output.field").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "反交错输出: 按场（倍帧率）");
  assert.deepEqual(node(host, "deinterlace"), { kind: "deinterlace", when: "interlaced", output: "field" });
  assert.equal(window.props["output-index"], 1);
  window.emit("choose", "deinterlace", 1);
  window.emit("choose", "output", 0);
  await host.settle();
  assert.deepEqual(node(host, "deinterlace"), { kind: "deinterlace", when: "all", output: "frame" });
  assert.equal(chain(host)[0]!.kind, "deinterlace", "always first");
  await stop();
});

test("E12: pad or crop to a ratio, move the picture in 5% steps, reset the position", async () => {
  const { host, window, stop } = await playing();
  await host.command("video.reframe.crop.4").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "裁剪/拉伸: 按 2.35:1 裁剪");
  assert.deepEqual(node(host, "reframe"), { kind: "reframe", mode: "crop", aspect: 2.35, x: 0, y: 0 });
  assert.equal(window.props["reframe-index"], 2);
  assert.equal(window.props["ratio-index"], 4);
  for (let i = 0; i < 3; i++) await host.command("video.reframe.up").run();
  await host.command("video.reframe.right").run();
  await host.settle();
  assert.deepEqual(host.osd.slice(-4), ["裁剪位置 (50%, 45%)", "裁剪位置 (50%, 40%)", "裁剪位置 (50%, 35%)", "裁剪位置 (55%, 35%)"]);
  const r = node(host, "reframe")!;
  assert.ok(Math.abs(r.x - 0.1) < 1e-9 && Math.abs(r.y + 0.3) < 1e-9);
  await host.command("video.reframe.reset").run();
  assert.equal(host.osd.at(-1), "裁剪位置 (50%, 50%)");
  window.emit("choose", "reframe", 1);
  window.emit("choose", "ratio", 0);
  await host.settle();
  assert.deepEqual(node(host, "reframe"), { kind: "reframe", mode: "pad", aspect: 1.3333, x: 0, y: 0 });
  assert.equal(host.command("video.reframe.pad.0").state.checked, true);
  await host.command("video.reframe.none").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "裁剪/拉伸: 不使用");
  assert.equal(node(host, "reframe"), undefined);
  // Before transform in the chain: the ratio is of the unrotated picture.
  await host.command("video.reframe.pad.1").run();
  await host.press("Alt+K");
  await host.settle();
  assert.deepEqual(kinds(host).slice(0, 3), ["deinterlace", "reframe", "transform"]);
  await stop();
});

// ---------------------------------------------------------------- settings

test("global settings and the last page survive a restart; flip and rotation do not", async () => {
  await withData(async (data) => {
    let { host, window, stop } = await playing(data);
    await host.press("O");
    await host.press("Ctrl+R");
    await host.press("Ctrl+P");
    await host.press("Ctrl+I");
    await host.command("video.reframe.pad.4").run();
    window.emit("tab-changed", 2);
    window.emit("commit", "edge", 70);
    await stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.version, 1);
    assert.equal(saved.picture.hue, 1);
    assert.equal(saved.tab, 2);
    ({ host, window, stop } = await playing(data));
    await host.settle();
    assert.equal(window.props["tab-index"], 2);
    assert.equal(window.props.hue, 1);
    assert.equal(window.props["sharpen-enabled"], true);
    assert.equal(window.props["edge-strength"], 70);
    assert.equal(window.props["flip-v"], false);
    assert.equal(host.command("video.deinterlace.on").state.checked, true);
    assert.equal(host.command("video.reframe.pad.4").state.checked, true);
    assert.deepEqual(kinds(host), ["deinterlace", "reframe", "eq", "hue", "sharpen"]);
    await stop();
  });
});

test("a broken settings file is kept aside and the defaults are used; an unknown version is left alone", async () => {
  await withData(async (data) => {
    await writeFile(join(data, "settings.json"), "{ not json");
    let { host, window, stop } = await playing(data);
    assert.equal(window.props.hue, 0);
    assert.ok((await readdir(data)).some((name) => name.startsWith("settings.json.bad-")));
    await stop();
    const future = JSON.stringify({ version: 2, picture: { hue: 50 } });
    await writeFile(join(data, "settings.json"), future);
    ({ host, window, stop } = await playing(data));
    assert.equal(window.props.hue, 0);
    await host.settle();
    await stop();
    assert.equal(await readFile(join(data, "settings.json"), "utf8"), future, "not rewritten until the user changes something");
  });
});

// ---------------------------------------------------------------- failures

test("headless: a missing on-screen display does not break the command or the window", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.handle("osd.show", () => {
    throw new HostError("unavailable", "headless");
  });
  const { stop } = await start(host);
  await host.command("video.picture").run();
  await host.press("E");
  await host.settle();
  assert.equal(eq(host).brightness, 0.01);
  assert.equal(host.window("picture").props.status, "");
  await stop();
});

test("empty media keeps edits and the ready source gets the latest values", async () => {
  const { host, stop } = await start();
  await host.command("video.picture").run();
  const window = host.window("picture");
  window.emit("adjust", "contrast", 35);
  await host.settle();
  assert.equal(host.sent("processing.replace").length, 0);
  assert.equal(window.props.status, "");
  host.openElsewhere("/movie.mp4");
  await host.settle();
  assert.equal(eq(host).contrast, 2 ** 0.35);
  assert.equal(latest(host).expectedGeneration, 1);
  await stop();
});

test("a refused change is shown and retried on request; a lost reply never sticks", async () => {
  const { host, window, stop } = await playing();
  host.handle("processing.replace", () => {
    throw new HostError("backend", "filter graph failed");
  });
  window.emit("adjust", "gamma", 20);
  await host.settle();
  assert.equal(window.props.status, "画面处理失败");
  assert.equal(window.props.failed, true);
  assert.equal(window.props.detail, "filter graph failed");
  host.reset("processing.replace");
  window.emit("retry");
  await host.settle();
  assert.equal(window.props.status, "");
  assert.equal(window.props.failed, false);
  // An audio-only source: the host refuses with `unsupported`.
  host.handle("processing.replace", () => {
    throw new HostError("unsupported", "no video stream");
  });
  window.emit("adjust", "gamma", 25);
  await host.settle();
  assert.equal(window.props.status, "当前来源没有画面");
  assert.deepEqual(host.osd, [], "no error on screen");
  host.reset("processing.replace");
  // The host applies the change but its reply is lost: the SDK resolves it.
  host.handle("processing.replace", async (params) => {
    host.reset("processing.replace");
    await host.request("processing.replace", params);
    throw new Error("reply lost");
  });
  window.emit("adjust", "gamma", 30);
  await host.settle();
  assert.equal(window.props.failed, false);
  const nodes = host.processing.resources.get("video:picture")!.nodes as Node[];
  assert.equal(nodes.find((n) => n.kind === "eq")!.gamma, 2 ** 0.3);
  await stop();
});

// ---------------------------------------------------------------- E18 倍帧

test("E18 model: 2 × the source rate up to 60, nothing from 50 fps, 60 for an unknown rate; frameBlend is last", () => {
  assert.equal(blendFps(24), 48);
  assert.equal(blendFps(23.976), 48);
  assert.equal(blendFps(25), 50);
  assert.equal(blendFps(29.97), 60);
  assert.equal(blendFps(10), 24, "the host's minimum");
  assert.equal(blendFps(null), 60);
  assert.equal(blendFps(50), null);
  assert.equal(blendFps(60), null);
  const on = defaultLook();
  on.effects.frameBlend = true;
  on.picture.blurEnabled = true;
  assert.deepEqual(nodesFor(on, 48).slice(-2), [{ kind: "gblur", sigma: 3, steps: 3 }, { kind: "frameBlend", fps: 48 }]);
  assert.equal(nodesFor(on, null).at(-1)!.kind, "gblur", "no rate (or enough): no node");
  on.effects.frameBlend = false;
  assert.equal(nodesFor(on, 48).at(-1)!.kind, "gblur");
});

test("E18: 倍帧 reads the source rate without frameBlend, then adds it last; a 60 fps source gets no node", async () => {
  const host = new TestHost();
  setFps(host, 24);
  host.openElsewhere("/pan24.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  await host.command("video.frame-blend").run();
  assert.equal(host.osd.at(-1), "倍帧: 开");
  assert.equal(host.command("video.frame-blend").state.checked, true);
  assert.equal(window.props["blend-enabled"], true);
  assert.equal(node(host, "frameBlend"), undefined, "not before the rate is read");
  await measured(host);
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 48 });
  assert.equal(window.props["blend-note"], "");
  // Now stats() reports the blended rate; a new 60 fps source is read again without the node.
  setFps(host, 60);
  host.openElsewhere("/sport60.mp4");
  await host.settle();
  assert.equal(node(host, "frameBlend"), undefined);
  await measured(host);
  assert.equal(node(host, "frameBlend"), undefined, "60 fps is smooth enough");
  assert.equal(window.props["blend-note"], "当前帧率已足够");
  assert.equal(host.command("video.frame-blend").state.checked, true, "the switch stays on");
  // Field output changes the base rate: read again.
  setFps(host, 25);
  await host.command("video.deinterlace.output.field").run();
  await measured(host);
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 50 });
  window.emit("toggle", "frameBlend", false);
  await host.settle();
  assert.equal(node(host, "frameBlend"), undefined);
  assert.equal(window.props["blend-note"], "");
  assert.equal(host.command("video.frame-blend").state.checked, false);
  await stop();
});

test("E18: an unknown rate blends to 60; no picture adds nothing; 启用画面效果 off removes it; the switch is saved", async () => {
  await withData(async (data) => {
    const host = new TestHost();
    setFps(host, undefined);
    host.openElsewhere("/song.mp3");
    let { stop } = await start(host, data);
    await host.command("video.frame-blend").run();
    await measured(host);
    assert.equal(node(host, "frameBlend"), undefined, "no picture yet");
    setFps(host, null);
    await measured(host);
    assert.equal(node(host, "frameBlend"), undefined, "an unknown rate is read again first");
    await new Promise((resolve) => setTimeout(resolve, 3000));
    await host.settle();
    assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 60 });
    await host.command("video.picture").run();
    host.window("picture").emit("toggle-effect", false);
    await host.settle();
    assert.deepEqual(chain(host), []);
    await stop();
    assert.equal(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).effects.frameBlend, true);
    ({ stop } = await start(host, data));
    assert.equal(host.command("video.frame-blend").state.checked, true);
    await stop();
  });
});

// ---------------------------------------------------------------- E11 宽高比与显示模式

test("E11 model: custom ratios parse as w:h or a decimal in 0.25–4; Shift+F6 cycles through the custom ratio once set", () => {
  assert.equal(parseAspect("21:9"), 2.3333);
  assert.equal(parseAspect(" 2.39 "), 2.39);
  assert.equal(parseAspect("4：3"), 1.3333);
  for (const bad of ["abc", "", "16:", ":9", "0:1", "16:9:1", "-2", "5", "1:5", "1e1"]) assert.equal(parseAspect(bad), null, bad);
  const display = { aspect: null as number | null, customAspect: "", fit: "keep" as const };
  const seen: string[] = [];
  for (let i = 0; i < 7; i++) {
    display.aspect = nextAspect(display);
    seen.push(aspectLabel(display));
  }
  assert.deepEqual(seen, ["4:3", "16:9", "16:10", "1.85:1", "2.35:1", "默认", "4:3"]);
  display.customAspect = "21:9";
  display.aspect = 2.35;
  display.aspect = nextAspect(display);
  assert.equal(aspectLabel(display), "自定义 - 21:9");
  assert.equal(aspectIndex(display), 6);
  assert.equal(nextAspect(display), null);
  const parsed = parse({ version: 1, view: { aspect: 2.1, customAspect: "abc", fit: "fill" } });
  assert.deepEqual(parsed.display, { aspect: 2.1, customAspect: "2.1", fit: "fill" }, "a lone ratio becomes the custom text");
  assert.deepEqual(parse({ version: 1, view: { aspect: 9, fit: "zoom" } }).display, { aspect: null, customAspect: "", fit: "keep" });
});

test("E11: 比例 menu and Shift+F6 set the display layer with PotPlayer's OSD; the chain is untouched", async () => {
  const { host, window, stop } = await playing();
  assert.deepEqual(host.videoView.values, { aspect: null, fit: "keep", zoom: 1, panX: 0, panY: 0 });
  const replaced = host.sent("processing.replace").length;
  await host.press("Shift+F6");
  await host.settle();
  assert.equal(host.osd.at(-1), "宽高比：4:3");
  assert.ok(Math.abs(host.videoView.values.aspect! - 4 / 3) < 1e-9);
  assert.equal(host.command("video.aspect.0").state.checked, true);
  assert.equal(window.props["aspect-index"], 1);
  await host.command("video.fit.stretch").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "不保持宽高比");
  assert.equal(host.videoView.values.fit, "stretch");
  assert.equal(window.props["fit-index"], 1);
  await host.command("video.fit.fill").run();
  assert.equal(host.osd.at(-1), "填满窗口");
  window.emit("choose", "aspect", 0);
  window.emit("choose", "fit", 0);
  await host.settle();
  assert.deepEqual(host.videoView.values, { aspect: null, fit: "keep", zoom: 1, panX: 0, panY: 0 });
  assert.equal(host.command("video.aspect.default").state.checked, true);
  assert.equal(host.osd.at(-1), "填满窗口", "window edits give no on-screen text");
  assert.equal(host.sent("processing.replace").length, replaced, "a display property, not a filter");
  await stop();
  assert.equal(host.videoView.owner, null, "released when the plugin stops");
});

test("E11: 自定义比例… only accepts a valid ratio, joins the menu and the Shift+F6 cycle, and is saved", async () => {
  await withData(async (data) => {
    let { host, stop } = await playing(data);
    await host.command("video.aspect.edit").run();
    const dialog = host.window("aspect");
    assert.equal(dialog.props.value, "");
    dialog.emit("edited", "abc");
    await host.settle();
    assert.equal(dialog.props.valid, false);
    assert.equal(dialog.props.error, "格式：16:9 或 2.39");
    dialog.emit("confirm", "abc");
    await host.settle();
    assert.ok(host.windows.has("aspect"), "an invalid ratio keeps the window open");
    dialog.emit("edited", "21:9");
    await host.settle();
    assert.equal(dialog.props.valid, true);
    assert.equal(dialog.props.error, "");
    dialog.emit("confirm", "21:9");
    await host.settle();
    assert.equal(host.windows.has("aspect"), false);
    assert.equal(host.osd.at(-1), "宽高比：自定义 - 21:9");
    assert.equal(host.videoView.values.aspect, 2.3333);
    const custom = host.command("video.aspect.custom");
    assert.equal(custom.declaration.path.at(-1), "自定义（21:9）");
    assert.equal(custom.state.checked, true);
    assert.deepEqual(host.window("picture").props["aspect-names"], ["默认（按来源）", "4:3", "16:9", "16:10", "1.85:1", "2.35:1", "自定义（21:9）"]);
    assert.equal(host.window("picture").props["aspect-index"], 6);
    await host.press("Shift+F6");
    assert.equal(host.osd.at(-1), "宽高比：默认");
    await host.command("video.aspect.4").run();
    await host.press("Shift+F6");
    assert.equal(host.osd.at(-1), "宽高比：自定义 - 21:9");
    await host.command("video.fit.fill").run();
    // Escape / 取消 closes without a change; reopening shows the saved text.
    await host.command("video.aspect.edit").run();
    assert.equal(host.window("aspect").props.value, "21:9");
    host.window("aspect").emit("cancel");
    await host.settle();
    assert.equal(host.windows.has("aspect"), false);
    await stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.deepEqual(saved.view, { aspect: 2.3333, customAspect: "21:9", fit: "fill" });
    ({ host, stop } = await playing(data));
    await host.settle();
    assert.deepEqual(host.videoView.values, { aspect: 2.3333, fit: "fill", zoom: 1, panX: 0, panY: 0 }, "restored at start");
    assert.equal(host.command("video.aspect.custom").state.checked, true);
    await stop();
  });
});

test("E11: another plugin owning the display layer: silent at start, a reason on use, taken once it is free", async () => {
  const host = new TestHost();
  host.setVideoViewOwner("probe.plugin");
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  await host.settle();
  const window = host.window("picture");
  assert.deepEqual(host.osd, []);
  assert.equal(window.props.status, "画面显示不可用");
  assert.equal(window.props.detail, "另一个插件正在控制画面显示");
  assert.equal(window.props.failed, true);
  await host.command("video.aspect.0").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "画面显示不可用：另一个插件正在控制画面显示");
  assert.equal(host.videoView.owner, "probe.plugin");
  host.setVideoViewOwner(null);
  window.emit("retry");
  await host.settle();
  assert.notEqual(host.videoView.owner, "probe.plugin");
  assert.ok(Math.abs(host.videoView.values.aspect! - 4 / 3) < 1e-9, "the chosen ratio is applied");
  assert.equal(window.props.failed, false);
  assert.equal(window.props.status, "");
  await stop();
});

// ---------------------------------------------------------------- E13 帧位

test("E13: keypad and Alt keys zoom by ×1.05 up to 400 % and pan by 1 %, with OSD; 5 alone does nothing", async () => {
  const { host, window, stop } = await playing();
  await host.press("Alt+Plus");
  await host.press("Numpad9");
  await host.settle();
  assert.deepEqual(host.osd, ["帧位调整：增加尺寸 (105%)", "帧位调整：增加尺寸 (110%)"]);
  assert.equal(host.videoView.values.zoom, 1.1025);
  assert.equal(window.props["zoom-text"], "缩放 110%");
  assert.ok(Math.abs((window.props["zoom-level"] as number) - Math.log2(1.1025)) < 1e-9);
  await host.press("Numpad1");
  assert.equal(host.osd.at(-1), "帧位调整：减少尺寸 (105%)");
  for (let i = 0; i < 3; i++) await host.press("Alt+ArrowLeft");
  assert.equal(host.osd.at(-1), "帧位调整：向左位移 (-3%, 0%)");
  await host.press("Ctrl+Numpad8");
  await host.press("Alt+ArrowDown");
  await host.press("Alt+ArrowDown");
  await host.press("Ctrl+Numpad6");
  await host.settle();
  assert.equal(host.osd.at(-1), "帧位调整：向右位移 (-2%, 1%)");
  assert.deepEqual([host.videoView.values.panX, host.videoView.values.panY], [-0.02, 0.01]);
  await host.press("Ctrl+Numpad5");
  await host.settle();
  assert.equal(host.osd.at(-1), "帧位调整：位移到中间");
  assert.deepEqual([host.videoView.values.zoom, host.videoView.values.panX, host.videoView.values.panY], [1.05, 0, 0]);
  for (let i = 0; i < 40; i++) await host.press("Numpad9");
  await host.settle();
  assert.equal(host.videoView.values.zoom, 4);
  assert.equal(host.osd.at(-1), "帧位调整：增加尺寸 (400%)", "the edge keeps the value and still reports it");
  await host.press("Numpad5");
  await host.settle();
  assert.equal(host.osd.at(-1), "帧位调整：重置");
  assert.equal(host.videoView.values.zoom, 1);
  await assert.rejects(host.press("5"), "the main keyboard's 5 is not 帧位复位");
  await stop();
});

test("E13: the 缩放 slider is logarithmic and follows the drag; 默认 resets; a new source starts unzoomed, silently", async () => {
  const { host, window, stop } = await playing();
  window.emit("adjust", "zoom", 1);
  await host.settle();
  assert.equal(host.videoView.values.zoom, 2);
  window.emit("commit", "zoom", -3);
  await host.settle();
  assert.equal(host.videoView.values.zoom, 0.25);
  window.emit("reset-field", "zoom");
  await host.settle();
  assert.equal(host.videoView.values.zoom, 1);
  assert.equal(window.props["zoom-level"], 0);
  await host.press("Alt+Plus");
  await host.press("Alt+ArrowUp");
  await host.command("video.aspect.1").run();
  const osd = host.osd.length;
  host.openElsewhere("/next.mp4");
  await host.settle();
  assert.equal(host.osd.length, osd, "the reset is silent");
  assert.equal(host.videoView.values.zoom, 1);
  assert.equal(host.videoView.values.panY, 0);
  assert.ok(Math.abs(host.videoView.values.aspect! - 16 / 9) < 1e-9, "the ratio is global and stays");
  await stop();
});

// ---------------------------------------------------------------- E16, E17, E18 运动补偿: GPU render options

const SOFTWARE = "需要 GPU 渲染（当前为软件渲染）";
/** `player.stats()` with a 1080p picture whose transfer is `transfer`. */
function setTransfer(host: TestHost, transfer: string | null) {
  setFps(host, 24);
  host.stats = { ...host.stats, video: { ...host.stats.video!, hwdec: "videotoolbox", transfer } };
}
const INVERT = "//!HOOK MAIN\n//!BIND HOOKED\nvec4 hook() { vec4 c = HOOKED_tex(HOOKED_pos); return vec4(1.0 - c.rgb, c.a); }\n";

test("render model: lenient parse, 目标亮度 in steps of 10, plain .glsl/.hook names, 倍帧 wins over 运动补偿", () => {
  assert.deepEqual(defaultLook().render, { toneMapping: "auto", targetPeak: 203, peakDetect: true, shadersEnabled: false, shaders: [], interpolation: false });
  const parsed = parse({
    version: 1,
    render: { toneMapping: "hable", targetPeak: 287, peakDetect: false, shadersEnabled: true, shaders: ["a.glsl", "../x.glsl", "a.glsl", ".h.glsl", "b.HOOK", 3, "c.txt"], interpolation: true },
  }).render;
  assert.deepEqual(parsed, { toneMapping: "hable", targetPeak: 290, peakDetect: false, shadersEnabled: true, shaders: ["a.glsl", "b.HOOK"], interpolation: true });
  assert.deepEqual(parse({ version: 1, render: { toneMapping: "aces", targetPeak: 9000 } }).render, { ...defaultLook().render, targetPeak: 400 });
  assert.equal(parse({ version: 1, effects: { frameBlend: true }, render: { interpolation: true } }).render.interpolation, false);
  assert.equal(shaderList(Array.from({ length: 20 }, (_, i) => `s${i}.glsl`)).length, 16);
  assert.deepEqual([peakOf(203), peakOf(96), peakOf(204), peakOf(401)], [203, 100, 200, 400]);
  assert.deepEqual(["pq", "hlg", "bt.1886", null].map(isHdr), [true, true, false, false]);
  assert.deepEqual(["x.glsl", "dir/x.glsl", "x", "x.glsl.txt"].map(isShaderName), [true, false, false, false]);
});

test("E16: tone mapping, 目标亮度 and peak detection reach the render options; SDR sources say they are not HDR", async () => {
  await withData(async (data) => {
    let host = new TestHost();
    setTransfer(host, "bt.1886");
    host.openElsewhere("/sdr.mp4");
    let { stop } = await start(host, data);
    await host.settle();
    assert.equal(host.videoRender.owner, "vestra.video");
    assert.deepEqual(host.videoRender.values, { toneMapping: "auto", targetPeak: 203, peakDetect: true, shaders: [], interpolation: false });
    await host.command("video.hdr.bt.2390").run();
    await host.settle();
    assert.equal(host.videoRender.values.toneMapping, "bt.2390");
    assert.equal(host.osd.at(-1), "HDR 色调映射: BT.2390（当前片源不是 HDR）");
    assert.equal(host.command("video.hdr.bt.2390").state.checked, true);
    await host.command("video.hdr.peak").run();
    await host.settle();
    assert.equal(host.videoRender.values.peakDetect, false);
    assert.equal(host.osd.at(-1), "动态峰值检测: 关");
    await host.command("video.hdr.settings").run();
    const window = host.window("picture");
    assert.equal(window.props["tab-index"], 3, "HDR 设置… opens the 高级 page");
    assert.equal(window.props["tone-index"], 2);
    assert.equal(window.props["hdr-note"], "目标亮度 203 尼特 · 当前片源不是 HDR");
    assert.equal(window.props["render-ready"], true);
    window.emit("adjust", "targetPeak", 296.4);
    await host.settle();
    assert.equal(host.videoRender.values.targetPeak, 300, "follows the drag in steps of 10");
    window.emit("choose", "toneMapping", 1);
    await host.settle();
    assert.equal(host.videoRender.values.toneMapping, "off");
    assert.equal(host.osd.at(-1), "动态峰值检测: 关", "window edits give no on-screen text");
    // An HDR10 source: no note.
    setTransfer(host, "pq");
    host.openElsewhere("/hdr10.mkv");
    await host.settle();
    assert.equal(window.props["hdr-note"], "目标亮度 300 尼特");
    await host.command("video.hdr.hable").run();
    assert.equal(host.osd.at(-1), "HDR 色调映射: Hable");
    await stop();
    assert.equal(host.videoRender.owner, null, "released when the plugin stops");
    assert.deepEqual(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).render, {
      toneMapping: "hable",
      targetPeak: 300,
      peakDetect: false,
      shadersEnabled: false,
      shaders: [],
      interpolation: false,
    });
    host = new TestHost();
    ({ stop } = await start(host, data));
    await host.settle();
    assert.deepEqual(host.videoRender.values, { toneMapping: "hable", targetPeak: 300, peakDetect: false, shaders: [], interpolation: false }, "restored at start");
    await stop();
  });
});

test("software rendering: the GPU items are disabled with the reason, the list and folder stay usable", async () => {
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  await host.settle();
  for (const id of ["video.shader.use", "video.shader.reload", "video.hdr.auto", "video.hdr.peak", "video.hdr.settings", "video.interpolation"]) {
    assert.equal(host.command(id).state.enabled, false, id);
    assert.equal(host.command(id).state.reason, SOFTWARE, id);
  }
  for (const id of ["video.shader.list", "video.shader.folder"]) assert.equal(host.command(id).state.enabled, true, id);
  const window = host.window("picture");
  assert.equal(window.props["render-ready"], false);
  assert.equal(window.props["render-reason"], SOFTWARE);
  assert.equal(window.props.status, "", "not a failure in the status line");
  assert.deepEqual(host.osd, []);
  await assert.rejects(host.press("Shift+P"), "disabled commands refuse their keys");
  await stop();
});

test("software rendering after start: the player's push disables the items", async () => {
  const { host, window, stop } = await playing();
  await host.settle();
  assert.equal(host.command("video.interpolation").state.enabled, true);
  host.failVideoRender("unsupported", "the window cannot show OpenGL frames");
  await host.settle();
  assert.equal(host.command("video.interpolation").state.enabled, false);
  assert.equal(host.command("video.interpolation").state.reason, SOFTWARE);
  assert.equal(window.props["render-ready"], false);
  await stop();
});

test("E17: 添加… copies the file into the shader folder; Shift+P sends absolute paths; the list is edited in the window", async () => {
  await withData(async (data) => {
    const desk = join(data, "desk");
    await mkdir(desk);
    await writeFile(join(desk, "invert.glsl"), INVERT);
    await writeFile(join(desk, "notes.txt"), "x");
    let host = new TestHost();
    host.openElsewhere("/movie.mp4");
    let { stop } = await start(host, data);
    await host.press("Shift+P");
    await host.settle();
    assert.equal(host.osd.at(-1), "像素着色: 没有着色器");
    assert.equal(host.command("video.shader.use").state.checked, true, "on, waiting for a shader");
    await host.command("video.shader.list").run();
    const list = host.window("shaders");
    host.dialogAnswers.push([join(desk, "notes.txt")]);
    list.emit("add");
    await host.settle();
    assert.equal(host.osd.at(-1), "只能添加 .glsl 或 .hook 着色器文件");
    host.dialogAnswers.push([join(desk, "invert.glsl")]);
    list.emit("add");
    await host.settle();
    const folder = join(data, "shaders");
    assert.equal(await readFile(join(folder, "invert.glsl"), "utf8"), INVERT, "copied into the folder");
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "invert.glsl")]);
    assert.deepEqual(list.props.names, ["invert.glsl"]);
    assert.equal(list.props.heading, "已启用 1/16（按顺序执行）");
    await rm(join(desk, "invert.glsl"));
    // Files dropped into the folder show below; double-click adds one.
    await writeFile(join(folder, "b.glsl"), INVERT);
    await writeFile(join(folder, "a.hook"), INVERT);
    await host.command("video.shader.list").run();
    await host.settle();
    assert.deepEqual(list.props.available, ["a.hook", "b.glsl"]);
    list.emit("pick", 1);
    await host.settle();
    assert.deepEqual(list.props.names, ["invert.glsl", "b.glsl"]);
    assert.deepEqual(list.props.available, ["a.hook"]);
    assert.equal(list.props.selected, 1);
    list.emit("up");
    await host.settle();
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "b.glsl"), join(folder, "invert.glsl")], "in list order");
    await host.press("Shift+P");
    await host.settle();
    assert.equal(host.osd.at(-1), "像素着色: 关");
    assert.deepEqual(host.videoRender.values.shaders, [], "off sends no shaders");
    assert.deepEqual(list.props.names, ["b.glsl", "invert.glsl"], "and keeps the list");
    assert.equal(list.props.note, "「使用着色器」未打开");
    await host.press("Shift+P");
    assert.equal(host.osd.at(-1), "像素着色: 开 (2)");
    list.emit("select", 1);
    list.emit("remove");
    await host.settle();
    assert.deepEqual(list.props.names, ["b.glsl"]);
    assert.deepEqual(list.props.available, ["a.hook", "invert.glsl"], "removed from the list, not from the folder");
    // 重新载入: a file deleted outside the player leaves the list with a note.
    await rm(join(folder, "b.glsl"));
    await host.command("video.shader.reload").run();
    await host.settle();
    assert.equal(host.osd.at(-1), "已移除不存在的着色器：b.glsl");
    assert.deepEqual(list.props.names, []);
    list.emit("pick", 1);
    await host.settle();
    await host.command("video.shader.folder").run();
    await host.settle();
    assert.deepEqual(host.system.opened, [folder]);
    await stop();
    host = new TestHost();
    ({ stop } = await start(host, data));
    await host.settle();
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "invert.glsl")], "list and switch restored");
    await stop();
  });
});

test("E17: a shader that does not compile shows in the status line and on screen; playback state is untouched", async () => {
  await withData(async (data) => {
    const folder = join(data, "shaders");
    await mkdir(folder);
    await writeFile(join(folder, "broken.glsl"), "//!HOOK MAIN\nvec4 hook() { return nope; }\n");
    await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, render: { shadersEnabled: true, shaders: ["broken.glsl"] } }));
    const host = new TestHost();
    host.openElsewhere("/movie.mp4");
    const { stop } = await start(host, data);
    await host.command("video.picture").run();
    await host.settle();
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "broken.glsl")]);
    host.failVideoRender("failed", "shader compile failed: ERROR: 0:2: Use of undeclared identifier 'nope'");
    await host.settle();
    const window = host.window("picture");
    assert.equal(window.props.status, "着色器载入失败");
    assert.equal(window.props.detail, "shader compile failed: ERROR: 0:2: Use of undeclared identifier 'nope'");
    assert.equal(window.props.failed, true);
    assert.equal(host.osd.at(-1), "着色器载入失败");
    assert.equal(host.command("video.shader.use").state.enabled, true, "still a GPU renderer");
    window.emit("toggle", "shaders", false);
    await host.settle();
    assert.equal(window.props.status, "");
    assert.deepEqual(host.videoRender.values.shaders, []);
    await stop();
  });
});

test("E18: 运动补偿 and 倍帧 exclude each other, with OSD; the window does the same", async () => {
  const host = new TestHost();
  setFps(host, 24);
  host.openElsewhere("/pan24.mp4");
  const { stop } = await start(host);
  await host.command("video.interpolation").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "运动补偿: 开");
  assert.equal(host.videoRender.values.interpolation, true);
  assert.equal(host.command("video.interpolation").state.checked, true);
  await host.command("video.frame-blend").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "倍帧: 开（已关闭运动补偿）");
  assert.equal(host.videoRender.values.interpolation, false);
  assert.equal(host.command("video.interpolation").state.checked, false);
  await measured(host);
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 48 });
  await host.command("video.picture").run();
  const window = host.window("picture");
  window.emit("toggle", "interpolation", true);
  await host.settle();
  assert.equal(host.osd.at(-1), "运动补偿: 开（已关闭倍帧）");
  assert.equal(node(host, "frameBlend"), undefined);
  assert.equal(window.props["blend-enabled"], false);
  assert.equal(window.props.interpolation, true);
  window.emit("toggle", "interpolation", false);
  await host.settle();
  assert.equal(host.videoRender.values.interpolation, false);
  assert.equal(host.osd.at(-1), "运动补偿: 开（已关闭倍帧）", "a plain window edit gives no on-screen text");
  await stop();
});

test("render options owned by another plugin: silent at start, a reason on use, taken once free", async () => {
  const host = new TestHost();
  host.setVideoRenderOwner("probe.plugin");
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  await host.settle();
  const window = host.window("picture");
  assert.deepEqual(host.osd, []);
  assert.equal(window.props.status, "渲染选项不可用");
  assert.equal(window.props.detail, "另一个插件正在控制渲染选项");
  assert.equal(host.command("video.interpolation").state.enabled, true, "a conflict is not software rendering");
  await host.command("video.hdr.clip").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "渲染选项不可用：另一个插件正在控制渲染选项");
  host.setVideoRenderOwner(null);
  window.emit("retry");
  await host.settle();
  assert.equal(host.videoRender.owner, "vestra.video");
  assert.equal(host.videoRender.values.toneMapping, "clip");
  assert.equal(window.props.status, "");
  await stop();
});
