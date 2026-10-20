import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";
import { defaultLook, defaults, droppedTooMany, nodesFor, parse, serialize } from "../model.ts";
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
  assert.deepEqual(parse(null), { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe, tab: 0 });
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
  assert.deepEqual(again, { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe });
});

test("field-output frame drops: more than 5 % of the frames since the last count", () => {
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 200, dropped: 4 }), false);
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 190, dropped: 10 }), true);
  assert.equal(droppedTooMany({ shown: 100, dropped: 5 }, { shown: 100, dropped: 5 }), false);
});

// ---------------------------------------------------------------- menu

test("menu under 视频 follows the chapter's allocation; deferred items are disabled with a reason", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  const declared = [...host.commands.values()].map(({ declaration: d }) => [d.path.slice(1).join(" ▶ "), d.order, d.group, d.bindings.join(",")]);
  const ratios = ["4:3", "16:9", "16:10", "1.85:1", "2.35:1"];
  assert.deepEqual(declared, [
    ["像素着色 ▶ 使用着色器", 402, "video-source", "Shift+P"],
    ["像素着色 ▶ HDR 色调映射", 402, "video-source", ""],
    ["反交错 ▶ 按序切换", 403, "video-source", "Ctrl+I"],
    ["反交错 ▶ 自动", 403, "video-source", ""],
    ["反交错 ▶ 总是使用", 403, "video-source", ""],
    ["反交错 ▶ 不使用", 403, "video-source", ""],
    ["反交错 ▶ 输出 ▶ 按帧", 403, "video-deinterlace-output", ""],
    ["反交错 ▶ 输出 ▶ 按场（倍帧率）", 403, "video-deinterlace-output", ""],
    ["裁剪/拉伸 ▶ 不使用", 405, "video-display", ""],
    ...ratios.map((r) => [`裁剪/拉伸 ▶ 按 ${r} 拉伸`, 405, "video-display", ""]),
    ...ratios.map((r) => [`裁剪/拉伸 ▶ 按 ${r} 裁剪`, 405, "video-display", ""]),
    ...["位置复位", "左移", "右移", "上移", "下移"].map((t) => [`裁剪/拉伸 ▶ ${t}`, 405, "video-reframe-position", ""]),
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
  for (const id of ["video.shader.use", "video.shader.hdr", "video.interpolation"])
    assert.deepEqual(host.command(id).state, { enabled: false, checked: null, busy: false, reason: "需要 GPU 渲染，当前版本不支持" });
  assert.equal(host.command("video.frame-blend").state.enabled, false);
  assert.match(host.command("video.frame-blend").state.reason!, /frameBlend/);
  await assert.rejects(host.press("Shift+P"));
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
