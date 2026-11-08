import assert from "node:assert/strict";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { readFileSync } from "node:fs";
import { isDeepStrictEqual } from "node:util";
import type { Locale } from "@vestra-player/sdk";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { aspectIndex, aspectMessage, blendFps, defaultLook, defaults, droppedTooMany, eqNeutral, isHdr, isShaderName, nextAspect, nodesFor, parse, parseAspect, peakOf, presets, serialize, shaderList } from "../model.ts";
import type { Look, Picture } from "../model.ts";

const directory = new URL("..", import.meta.url);
/** English (the source texts) unless a locale is given; `zh` is what a Simplified Chinese user sees. */
const zh: Locale = "zh-Hans";
const start = (host = new TestHost(), dataDirectory?: string, locale: Locale = "en") =>
  startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
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
/** Deinterlace Auto: the player's per-frame deinterlacer, always at field rate (not a filter). */
const autoDeinterlace = { kind: "deinterlace", when: "auto", output: "field" };
/** The chain the TestHost accepted (it validates nodes as the player does). */
const applied = (host: TestHost) => [...host.processing.resources.values()].find((r) => r.stage === "video")?.nodes as Node[] | undefined;
async function playing(dataDirectory?: string, locale: Locale = "en") {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const started = await start(host, dataDirectory, locale);
  await host.command("video.picture").run();
  return { ...started, host, window: host.mounted() };
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
/**
 * Poll an observable result, then settle. `host.settle()` waits only for host calls, not for
 * the plugin's own file-system work (shader folder scan and copy) or timers (the source-rate
 * read), so assert after those this way. The settle afterwards matters: `host.sent` records a
 * request before the host handles it, and window properties arrive one call each.
 */
async function until(host: TestHost, predicate: () => boolean | Promise<boolean>, what = "condition") {
  for (let i = 0; i < 1200; i++) {
    if (await predicate()) return host.settle();
    await new Promise((done) => setTimeout(done, 5));
  }
  assert.fail(`timed out waiting for ${what}`);
}
/** Wait until `read()` deep-equals `expected` (the value asserted is the value waited on). */
async function eventually(read: () => unknown, expected: unknown, message?: string) {
  let last: unknown;
  for (let i = 0; i < 1200; i++) {
    last = read();
    if (isDeepStrictEqual(last, expected)) return;
    await new Promise((done) => setTimeout(done, 5));
  }
  assert.deepStrictEqual(last, expected, message);
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const blendNode = (host: TestHost, fps: number) => () => same(chain(host).at(-1), { kind: "frameBlend", fps });
async function withData(run: (data: string) => Promise<void>) {
  const data = await mkdtemp(join(tmpdir(), "vestra-video-"));
  try {
    await run(data);
  } finally {
    await rm(data, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------- model

test("neutral: automatic deinterlace only; eq, blur, hue and strengths add nodes only when they act", () => {
  assert.deepEqual(nodesFor(defaultLook()), [autoDeinterlace]);
  assert.deepEqual(nodesFor(look({ blurEnabled: true, blurStrength: 4 })).at(-1), { kind: "gblur", sigma: 4, steps: 3 });
  assert.equal(nodesFor(look({ blurEnabled: true, blurStrength: 0 })).length, 1);
  assert.deepEqual(nodesFor(look({ blurEnabled: true, blurStrength: 15 })).at(-1), { kind: "gblur", sigma: 10, steps: 3 });
  assert.deepEqual(nodesFor(look({ hue: 100 })).at(-1), { kind: "hue", degrees: 180 }, "E02: 1% = 1.8°");
  assert.deepEqual(nodesFor(look({ hue: -6 })).at(-1), { kind: "hue", degrees: -10.8 });
  const zero = defaultLook();
  zero.effects.sharpen = { on: true, strength: 0 };
  zero.effects.deband = { on: true, strength: 0 };
  assert.deepEqual(nodesFor(zero), [autoDeinterlace], "on at strength 0 makes no node");
  assert.deepEqual(nodesFor({ ...defaultLook(), deinterlace: { mode: "off", output: "frame" } }), [], "nothing active: an empty chain");
});

test("deinterlace: Auto is the player's deinterlacer at field rate whatever Output says; Always is bwdif with Output; Never nothing", () => {
  const chainFor = (mode: "auto" | "on" | "off", output: "frame" | "field") => nodesFor({ ...defaultLook(), deinterlace: { mode, output } });
  assert.deepEqual(defaultLook().deinterlace, { mode: "auto", output: "frame" }, "the stored default is unchanged");
  assert.deepEqual(chainFor("auto", "frame"), [autoDeinterlace], "Auto + By Frame: still field rate (frame is refused with auto)");
  assert.deepEqual(chainFor("auto", "field"), [autoDeinterlace]);
  assert.deepEqual(chainFor("on", "frame"), [{ kind: "deinterlace", when: "all", output: "frame" }]);
  assert.deepEqual(chainFor("on", "field"), [{ kind: "deinterlace", when: "all", output: "field" }]);
  assert.deepEqual(chainFor("off", "field"), []);
  const auto = look({ brightness: 10 }, { deinterlace: { mode: "auto", output: "frame" } });
  const nodes = nodesFor(auto);
  assert.deepEqual(nodes[0], autoDeinterlace, "first, with colour nodes after it");
  assert.equal(nodes.filter((n) => n.kind === "deinterlace").length, 1, "never a second deinterlace node next to auto");
  assert.ok(!nodes.some((n) => n.kind === "deinterlace" && n.when !== "auto"), "no bwdif under Auto");
});

test("the eq node exists only while a colour differs from its default; `keepEq` keeps a neutral one", () => {
  assert.equal(eqNeutral(defaults), true);
  assert.deepEqual(nodesFor(look({ brightness: 1 })), [autoDeinterlace, { ...neutralEq, brightness: 0.01 }]);
  assert.deepEqual(nodesFor(look({ contrast: 15 })).at(-1), { ...neutralEq, contrast: 2 ** 0.15 });
  assert.deepEqual(nodesFor(look({ saturation: -12 })).at(-1), { ...neutralEq, saturation: 0.88 });
  assert.deepEqual(nodesFor(look({ gamma: -5 })).at(-1), { ...neutralEq, gamma: 2 ** -0.05 });
  assert.deepEqual(nodesFor(look({ hue: 3 })), [autoDeinterlace, { kind: "hue", degrees: 5.4 }], "hue has its own node; no eq");
  assert.deepEqual(nodesFor(look({ blurEnabled: true })), [autoDeinterlace, { kind: "gblur", sigma: 3, steps: 3 }]);
  assert.deepEqual(nodesFor(look({ brightness: 0.4 })).at(-1), { ...neutralEq, brightness: 0.004 }, "a fraction from a drag counts");
  assert.deepEqual(nodesFor(look(), null, true), [autoDeinterlace, neutralEq]);
  assert.deepEqual(nodesFor(look({ hue: 3 }), null, true), [autoDeinterlace, neutralEq, { kind: "hue", degrees: 5.4 }], "eq first among the colour nodes");
  assert.deepEqual(nodesFor(look(presets.original)), [autoDeinterlace]);
  assert.deepEqual(nodesFor(look(presets.vivid)).at(-1), { ...neutralEq, contrast: 2 ** 0.15, saturation: 1.2 });
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
  assert.deepEqual(parse(null), { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe, display: base.display, render: base.render, advanced: false });
  const parsed = parse({
    version: 1,
    picture: { brightness: 500, hue: 12.6, blurEnabled: true, blurStrength: "x" },
    effects: { sharpen: { on: true, strength: 70 }, denoise: { temporal: { on: true, strength: 300 }, shown: "temporal" }, deblock: 4 },
    deinterlace: { mode: "off", output: "sideways" },
    reframe: { mode: "pad", ratio: 9, x: 120, y: 10 },
    tab: 3,
    advanced: true,
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
  assert.equal(parsed.advanced, true);
  assert.equal("tab" in parsed, false, "the old window page is not read");
  assert.equal(parse({ version: 1, advanced: "yes" }).advanced, false);
  assert.equal(parse({ version: 2, picture: { brightness: 40 }, advanced: true }).picture.brightness, 0);
  const written = serialize({ ...base, geometry: { flipH: true, flipV: true, rotate: 90 } }, true);
  assert.equal(written.version, 1);
  assert.equal("geometry" in written, false, "flip and rotation are per source");
  const { advanced, ...again } = parse(JSON.parse(JSON.stringify(written)));
  assert.equal(advanced, true);
  assert.deepEqual(again, { picture: base.picture, effects: base.effects, deinterlace: base.deinterlace, reframe: base.reframe, display: base.display, render: base.render });
});

test("field-output frame drops: more than 5 % of the frames since the last count", () => {
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 200, dropped: 4 }), false);
  assert.equal(droppedTooMany({ shown: 100, dropped: 0 }, { shown: 190, dropped: 10 }), true);
  assert.equal(droppedTooMany({ shown: 100, dropped: 5 }, { shown: 100, dropped: 5 }), false);
});

// ---------------------------------------------------------------- commands and menu

test("only Picture… is in the menu (Alt+1); every other command is in the palette with its order, group and keys", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  const ratios = ["4:3", "16:9", "16:10", "1.85:1", "2.35:1"];
  const expected = [
    ["Use Shaders", 402, "video-source", "Shift+P"],
    ["Shader List…", 402, "video-source", ""],
    ["Open Shader Folder", 402, "video-source", ""],
    ["Reload Shaders", 402, "video-source", ""],
    ...["Auto", "Off", "BT.2390", "Hable", "Mobius", "Reinhard", "Clip"].map((t) => [t, 402, "video-hdr", ""]),
    ["Dynamic Peak Detection", 402, "video-hdr-peak", ""],
    ["HDR Settings…", 402, "video-hdr", ""],
    ["Cycle", 403, "video-source", "Ctrl+I"],
    ["Auto", 403, "video-source", ""],
    ["Always", 403, "video-source", ""],
    ["Never", 403, "video-source", ""],
    ["By Frame", 403, "video-deinterlace-output", ""],
    ["By Field (Double Rate)", 403, "video-deinterlace-output", ""],
    ["Keep Aspect Ratio", 404, "video-display", ""],
    ["Ignore Aspect Ratio", 404, "video-display", ""],
    ["Fill Window (Crop Excess)", 404, "video-display", ""],
    ["Cycle Aspect Ratio", 404, "video-aspect", "Shift+F6"],
    ["Default (Source)", 404, "video-aspect", ""],
    ...ratios.map((r) => [r, 404, "video-aspect", ""]),
    ["Custom (x:y)", 404, "video-aspect", ""],
    ["Custom Aspect Ratio…", 404, "video-aspect", ""],
    ["None", 405, "video-display", ""],
    ...ratios.map((r) => [`Stretch to ${r}`, 405, "video-display", ""]),
    ...ratios.map((r) => [`Crop to ${r}`, 405, "video-display", ""]),
    ...["Move Left", "Move Right", "Move Up", "Move Down"].map((t) => [t, 405, "video-reframe-position", ""]),
    ["Crop Position", 442, "reset-video", ""],
    // The mpv-style Alt keys come first: the panel's key caps show the first key.
    ["Zoom In", 406, "video-display", "Alt+Plus,Numpad9"],
    ["Zoom Out", 406, "video-display", "Alt+-,Numpad1"],
    ["Pan →", 406, "video-frame-pan", "Alt+ArrowRight,Ctrl+Numpad6"],
    ["Pan ←", 406, "video-frame-pan", "Alt+ArrowLeft,Ctrl+Numpad4"],
    ["Pan ↑", 406, "video-frame-pan", "Alt+ArrowUp,Ctrl+Numpad8"],
    ["Pan ↓", 406, "video-frame-pan", "Alt+ArrowDown,Ctrl+Numpad2"],
    ["Center Frame", 406, "video-frame-pan", "Ctrl+Numpad5"],
    ["Framing", 441, "reset-video", "Alt+Backspace,Numpad5"],
    ["Flip Vertically", 408, "video-effects", "Ctrl+P"],
    ["Flip Horizontally", 408, "video-effects", "Ctrl+Z"],
    ["Gaussian Blur", 408, "video-sharpness", "Ctrl+B"],
    ["Sharpen", 408, "video-sharpness", "Ctrl+R"],
    ["Edge Sharpen", 408, "video-sharpness", ""],
    ["Deblock", 408, "video-cleanup", "Ctrl+H"],
    ["Deband", 408, "video-cleanup", ""],
    ["3D Denoise", 408, "video-cleanup", "Ctrl+N"],
    ["Gradual Denoise", 408, "video-cleanup", ""],
    ["Temporal Denoise", 408, "video-cleanup", "Ctrl+M"],
    ["Frame Doubling", 408, "video-frames", ""],
    ["Motion Interpolation", 408, "video-frames", ""],
    ["Cycle", 409, "video-effects", "Alt+K"],
    ["No Rotation", 409, "video-effects", ""],
    ["90°", 409, "video-effects", ""],
    ["180°", 409, "video-effects", ""],
    ["270°", 409, "video-effects", ""],
    ["Picture…", 796, "windows", "Alt+1"],
    ["Picture", 440, "reset-video", "Q"],
    ["Picture Panel", 443, "reset-video", ""],
    ["Brightness -1%", 442, "video-color", "W"],
    ["Brightness +1%", 443, "video-color", "E"],
    ["Contrast -1%", 444, "video-color", "R"],
    ["Contrast +1%", 445, "video-color", "T"],
    ["Saturation -1%", 446, "video-color", "Y"],
    ["Saturation +1%", 447, "video-color", "U"],
    ["Hue -1%", 448, "video-color", "I"],
    ["Hue +1%", 449, "video-color", "O"],
    ["GPU Rendering (Experimental)", 460, "video-rendering", ""],
  ];
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d, display }) => [display.title, d.order, d.group, d.bindings.join(",")]),
    expected,
  );
  // The Video ▸ submenu and Reset ▸ Picture / Framing / Crop Position are gone: no path but Picture…'s.
  const paths = [...host.commands.entries()].filter(([, { declaration: d }]) => d.path.length > 0).map(([id, { declaration: d }]) => [id, d.path]);
  assert.deepEqual(paths, [["video.picture", ["Picture…"]]], "Picture… at the top of the context menu");
  assert.deepEqual(host.conflicts, []);
  await host.settle();
  for (const id of ["video.shader.use", "video.interpolation"])
    assert.deepEqual(host.command(id).state, { enabled: true, checked: false, busy: false, reason: null });
  assert.equal(host.command("video.hdr.auto").state.checked, true);
  assert.equal(host.command("video.hdr.peak").state.checked, true);
  assert.deepEqual(host.command("video.frame-blend").state, { enabled: true, checked: false, busy: false, reason: null });
  assert.equal(host.command("video.aspect.custom").state.enabled, false, "Custom waits for a ratio");
  assert.equal(host.command("video.fit.keep").state.checked, true);
  assert.equal(host.command("video.aspect.default").state.checked, true);
  assert.equal(host.command("video.deinterlace.auto").state.checked, true);
  assert.equal(host.command("video.deinterlace.output.frame").state.checked, true);
  for (const id of ["video.deinterlace.output.frame", "video.deinterlace.output.field"])
    assert.deepEqual(
      host.command(id).state,
      { enabled: false, checked: id.endsWith("frame"), busy: false, reason: "Auto outputs by field (double rate); this setting applies to Always" },
      "Output is greyed under Auto, with the reason",
    );
  assert.equal(host.command("video.reframe.none").state.checked, true);
  assert.equal(host.command("video.rotate.0").state.checked, true);
  await stop();
});

test("Picture… opens the adjust column at the Picture tab and closes it again (Alt+1); no window", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  try {
    await host.settle();
    const panel = host.mounted();
    assert.equal(panel.slot, "adjust");
    assert.equal(panel.component, "Picture");
    assert.deepEqual(host.layout.adjust?.panels?.map((p) => p.title), ["Picture"]);
    assert.equal(host.layout.adjust?.preferred, false, "the column starts closed");
    await host.press("Alt+1");
    await until(host, () => host.layout.adjust?.preferred === true, "the column to open");
    assert.equal(host.layout.adjust?.owner, 1, "at the Picture tab");
    await host.command("video.picture").run();
    await until(host, () => host.layout.adjust?.preferred === false, "the column to close");
    assert.equal(host.sent("ui.open").length, 0, "the Picture Adjustments window is gone");
  } finally {
    await stop();
  }
});

test("every command has a category for the command palette, and names Picture… as its home", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  const commands = [...host.commands.entries()];
  assert.equal(commands.length, 84);
  for (const [id, command] of commands) assert.ok(command.display.category, `${id} has a category`);
  const count = new Map<string, number>();
  for (const [, { display }] of commands) count.set(display.category!, (count.get(display.category!) ?? 0) + 1);
  assert.deepEqual(Object.fromEntries(count), {
    "Pixel Shaders": 5,
    "HDR Tone Mapping": 8,
    Deinterlace: 6,
    Aspect: 12,
    "Crop/Stretch": 15,
    Reset: 4,
    Framing: 7,
    "Image Processing": 12,
    Rotation: 5,
    Picture: 10,
  });
  // The palette names them as the menu did: the old submenu, "Reset" for the old Reset ▸ items.
  const category = (id: string) => host.command(id).display.category;
  assert.equal(category("video.deinterlace.output.field"), "Deinterlace");
  assert.equal(category("video.hdr.peak"), "HDR Tone Mapping");
  assert.equal(category("video.hdr.settings"), "Pixel Shaders");
  for (const id of ["video.reset", "video.resetPanel", "video.frame.reset", "video.reframe.reset"]) assert.equal(category(id), "Reset", id);
  for (const id of ["video.picture", "video.brightness.up", "video.gpu-rendering"]) assert.equal(category(id), "Picture", id);
  // Every control is in the panel that Picture… opens; the shader folder belongs to the Shader List window.
  const homes = new Map<string, string[]>();
  for (const [id, { declaration: d }] of commands) if (d.home) homes.set(d.home, [...(homes.get(d.home) ?? []), id]);
  assert.deepEqual([...homes.keys()].sort(), ["video.picture", "video.shader.list"]);
  assert.deepEqual(homes.get("video.shader.list"), ["video.shader.folder"]);
  const without = commands.filter(([, { declaration: d }]) => !d.home).map(([id]) => id);
  assert.deepEqual(without, ["video.shader.reload", "video.picture"], "Reload Shaders has no control; Picture… opens the panel");
  await stop();
});

test("Simplified Chinese: the palette names commands with the menu's translations", async () => {
  const host = new TestHost();
  const { stop } = await start(host, undefined, zh);
  const shown = [...host.commands.values()].map(({ display }) => `${display.category}: ${display.title}`);
  const ratios = ["4:3", "16:9", "16:10", "1.85:1", "2.35:1"];
  assert.deepEqual(shown, [
    "像素着色: 使用着色器",
    "像素着色: 着色器列表…",
    "像素着色: 打开着色器文件夹",
    "像素着色: 重新载入着色器",
    ...["自动", "关", "BT.2390", "Hable", "Mobius", "Reinhard", "裁剪"].map((t) => `HDR 色调映射: ${t}`),
    "HDR 色调映射: 动态峰值检测",
    "像素着色: HDR 设置…",
    "反交错: 按序切换",
    "反交错: 自动",
    "反交错: 总是使用",
    "反交错: 不使用",
    "反交错: 按帧",
    "反交错: 按场（倍帧率）",
    "比例: 保持宽高比",
    "比例: 不保持宽高比",
    "比例: 填满窗口（裁去多余）",
    "比例: 依次切换比例",
    "比例: 默认（按来源）",
    ...ratios.map((r) => `比例: ${r}`),
    "比例: 自定义（x:y）",
    "比例: 自定义比例…",
    "裁剪/拉伸: 不使用",
    ...ratios.map((r) => `裁剪/拉伸: 按 ${r} 拉伸`),
    ...ratios.map((r) => `裁剪/拉伸: 按 ${r} 裁剪`),
    ...["左移", "右移", "上移", "下移"].map((t) => `裁剪/拉伸: ${t}`),
    "复位: 裁剪位置",
    "帧位: 全帧 +",
    "帧位: 全帧 -",
    "帧位: 偏移 →",
    "帧位: 偏移 ←",
    "帧位: 偏移 ↑",
    "帧位: 偏移 ↓",
    "帧位: 帧位居中",
    "复位: 帧位",
    ...["上下翻转", "左右翻转", "高斯模糊", "锐化", "边缘锐化", "去块", "消色带", "3D 降噪", "渐进降噪", "瞬态降噪", "倍帧", "运动补偿"].map((t) => `图像处理: ${t}`),
    ...["按序切换", "不旋转", "90°", "180°", "270°"].map((t) => `图像旋转: ${t}`),
    "画面: 画面…",
    "复位: 画面",
    "复位: 画面面板",
    ...["亮度", "对比度", "饱和度", "色调"].flatMap((t) => [`画面: ${t} -1%`, `画面: ${t} +1%`]),
    "画面: GPU 渲染（实验性）",
  ]);
  assert.deepEqual(host.command("video.picture").display.path, ["画面…"]);
  // The player translates the reason with the plugin's catalog.
  assert.equal(host.translate(host.command("video.aspect.custom").state.reason!), "先用「自定义比例…」输入");
  await stop();
});

test("every text of the plugin is translated into Simplified Chinese", () => {
  const read = (file: string) => readFileSync(new URL(`../locales/${file}`, import.meta.url), "utf8");
  const ids = [...read("messages.pot").matchAll(/^msgid "(.+)"$/gm)].map((m) => m[1]);
  assert.ok(ids.includes("Picture…") && ids.includes("Enabled {}/{} (run in order)"));
  // Ratios are not texts.
  assert.ok(!ids.includes("16:9"));
  const po = read("zh-Hans.po");
  assert.doesNotMatch(po, /msgstr(\[0\])? ""\n\n/);
  assert.match(po, /msgid "Needs GPU rendering \(currently software rendering\)"\nmsgstr "需要 GPU 渲染（当前为软件渲染）"/);
  assert.match(po, /msgid "\{\} nits"\nmsgstr "\{\} 尼特"/);
  assert.match(po, /msgid "Picture…"\nmsgstr "画面…"/);
  // The other five languages are complete too (machine translations are marked x-machine).
  for (const lang of ["de", "es", "fr", "ja", "ko"]) assert.doesNotMatch(read(`${lang}.po`), /msgstr(\[0\])? ""\n\n/, lang);
});

// ---------------------------------------------------------------- colour

test("colour keys step colours by 1% with on-screen feedback; Q resets; Ctrl+B toggles blur", async () => {
  const { host, window, stop } = await playing();
  for (const key of ["E", "E", "W", "T", "Y"]) await host.press(key);
  await host.settle();
  assert.deepEqual(host.osd, ["Brightness +1% (+1%)", "Brightness +1% (+2%)", "Brightness -1% (+1%)", "Contrast +1% (+1%)", "Saturation -1% (-1%)"]);
  assert.equal(eq(host).brightness, 0.01);
  assert.equal(eq(host).contrast, 2 ** 0.01);
  assert.equal(eq(host).saturation, 0.99);
  await host.press("R");
  await host.press("U");
  assert.equal(host.osd.at(-1), "Saturation +1% (0%)");
  // A dragged fractional value steps from its rounded value; limits hold.
  window.emit("adjust", "brightness", 99.6);
  await host.settle();
  await host.press("E");
  await host.press("E");
  assert.equal(window.props.brightness, 100);
  assert.equal(host.osd.at(-1), "Brightness +1% (+100%)");
  await host.press("Ctrl+B");
  await host.settle();
  assert.equal(host.osd.at(-1), "Gaussian blur: on");
  assert.equal(host.command("video.gaussian").state.checked, true);
  assert.equal(window.props["blur-enabled"], true);
  assert.equal(kinds(host).at(-1), "gblur");
  await host.press("Ctrl+B");
  assert.equal(host.osd.at(-1), "Gaussian blur: off");
  await host.press("Q");
  await host.settle();
  assert.equal(host.osd.at(-1), "Picture reset");
  assert.deepEqual(chain(host), nodesFor(defaultLook()));
  assert.equal(window.props.brightness, 0);
  await stop();
});

test("E02: I / O step the hue, the window follows, Q and 原始 reset it; other presets keep it at 0", async () => {
  const { host, window, stop } = await playing(undefined, zh);
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

// The eq node is added and removed while playing: with the colours at their defaults there is none,
// so an untouched picture keeps zero-copy hardware decoding.
test("defaults: no eq node; one colour away from its default adds it with exact values; back to the default removes it", async () => {
  const { host, window, stop } = await playing();
  assert.deepEqual(kinds(host), ["deinterlace"], "defaults: no eq node");
  assert.equal(node(host, "eq"), undefined);
  await host.press("E");
  await host.settle();
  assert.deepEqual(chain(host), [autoDeinterlace, { kind: "eq", brightness: 0.01, contrast: 1, saturation: 1, gamma: 1 }], "one value: the node, exact");
  await host.press("W");
  await host.settle();
  assert.deepEqual(kinds(host), ["deinterlace"], "back at the default: removed");
  window.emit("commit", "gamma", -5);
  await host.settle();
  assert.deepEqual(eq(host), { kind: "eq", brightness: 0, contrast: 1, saturation: 1, gamma: 2 ** -0.05 });
  window.emit("reset-field", "gamma");
  await host.settle();
  assert.equal(node(host, "eq"), undefined, "the row's 默认 button removes it");
  window.emit("preset", "vivid");
  await host.settle();
  assert.deepEqual(eq(host), { kind: "eq", brightness: 0, contrast: 2 ** 0.15, saturation: 1.2, gamma: 1 });
  window.emit("preset", "original");
  await host.settle();
  assert.equal(node(host, "eq"), undefined, "the 原始 preset removes it");
  // Hue and blur have their own nodes: with the colours at their defaults they come without eq.
  await host.press("O");
  await host.press("Ctrl+B");
  await host.settle();
  assert.deepEqual(kinds(host), ["deinterlace", "hue", "gblur"]);
  await stop();
});

test("Reset (Q) removes the eq node; so does the menu command", async () => {
  const { host, window, stop } = await playing();
  window.emit("adjust", "contrast", 30);
  window.emit("adjust", "saturation", -20);
  await host.settle();
  assert.deepEqual(eq(host), { kind: "eq", brightness: 0, contrast: 2 ** 0.3, saturation: 0.8, gamma: 1 });
  await host.press("Q");
  await host.settle();
  assert.equal(host.osd.at(-1), "Picture reset");
  assert.deepEqual(kinds(host), ["deinterlace"], "reset: removed");
  window.emit("adjust", "brightness", 12);
  await host.settle();
  assert.equal(eq(host).brightness, 0.12);
  await host.command("video.reset").run();
  await host.settle();
  assert.equal(node(host, "eq"), undefined);
  // With deinterlace off the chain is empty: nothing left for the player to decode copy-back for.
  await host.command("video.deinterlace.off").run();
  await host.settle();
  assert.deepEqual(chain(host), []);
  await stop();
});

test("Reset (Q) also turns every image-processing switch off, keeps strengths and geometry, and empties the chain", async () => {
  await withData(async (data) => {
    const host = new TestHost();
    setFps(host, 24);
    host.openElsewhere("/movie.mp4");
    const { stop } = await start(host, data);
    await host.command("video.picture").run();
    const window = host.mounted();
    await host.command("video.deinterlace.off").run();
    await host.command("video.reframe.crop.1").run();
    await host.press("Ctrl+P");
    await host.press("E");
    await host.press("Ctrl+B");
    for (const id of ["video.sharpen", "video.edge", "video.denoise.d3", "video.denoise.gradual", "video.denoise.temporal", "video.deblock", "video.deband", "video.frame-blend"])
      await host.command(id).run();
    window.emit("commit", "denoise", 90);
    await until(host, blendNode(host, 48), "the 24 fps rate read");
    assert.deepEqual(kinds(host), ["reframe", "transform", "denoise", "denoise", "denoise", "deblock", "deband", "eq", "sharpen", "sharpen", "gblur", "frameBlend"]);
    await host.press("Q");
    await host.settle();
    assert.equal(host.osd.at(-1), "Picture reset; image processing off");
    assert.deepEqual(kinds(host), ["reframe", "transform"], "flip and crop have their own resets");
    for (const id of ["video.gaussian", "video.sharpen", "video.edge", "video.denoise.d3", "video.denoise.gradual", "video.denoise.temporal", "video.deblock", "video.deband", "video.frame-blend"])
      assert.equal(host.command(id).state.checked, false, id);
    assert.equal(host.command("video.flip.vertical").state.checked, true);
    assert.equal(window.props["denoise-enabled"], false);
    assert.equal(window.props["blend-enabled"], false);
    assert.equal(window.props["denoise-strength"], 90, "off keeps the strength");
    // Nothing of this plugin left on: no filter, so zero-copy decoding can resume.
    await host.press("Ctrl+P");
    await host.command("video.reframe.none").run();
    await host.settle();
    assert.deepEqual(chain(host), []);
    // A switch turned back on comes back at its strength; a second Q with nothing on says the short text.
    await host.press("Ctrl+N");
    await host.settle();
    assert.equal(chain(host)[0]!.lumaSpatial, 9);
    await host.press("Q");
    await host.press("Q");
    await host.settle();
    assert.equal(host.osd.at(-1), "Picture reset");
    await stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.effects.denoise.d3.on, false, "the reset is saved");
    assert.equal(saved.effects.denoise.d3.strength, 90);
    assert.equal(saved.effects.frameBlend, false);
  });
});

test("Reset (Q) turns motion interpolation off too, with the translated text", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host, undefined, zh);
  await host.command("video.interpolation").run();
  await host.settle();
  assert.equal(host.videoRender.values.interpolation, true);
  await host.press("Q");
  await host.settle();
  assert.equal(host.osd.at(-1), "画面调节已复位，图像处理已关闭");
  assert.equal(host.videoRender.values.interpolation, false);
  assert.equal(host.command("video.interpolation").state.checked, false);
  await stop();
});

test("a drag keeps the eq node through 0 and the release removes it; a drag that stays at the default adds none", async () => {
  const { host, window, stop } = await playing();
  const sent = () => host.sent("processing.replace").length;
  // Moving a slider to exactly its default adds nothing.
  const before = sent();
  window.emit("adjust", "brightness", 0);
  window.emit("commit", "brightness", 0);
  await host.settle();
  assert.equal(sent(), before, "no change sent");
  assert.equal(node(host, "eq"), undefined);
  // Away from the default: the node appears and follows the drag.
  window.emit("adjust", "brightness", 5);
  await host.settle();
  assert.equal(eq(host).brightness, 0.05);
  // Through 0 on the way down: the node stays, neutral, so the player updates it in place.
  window.emit("adjust", "brightness", 0);
  await host.settle();
  assert.deepEqual(eq(host), neutralEq, "kept during the drag");
  window.emit("adjust", "brightness", -3);
  await host.settle();
  assert.equal(eq(host).brightness, -0.03);
  // Released at 0: removed.
  window.emit("adjust", "brightness", 0);
  window.emit("commit", "brightness", 0);
  await host.settle();
  assert.equal(node(host, "eq"), undefined, "the release removes it");
  assert.deepEqual(kinds(host), ["deinterlace"]);
  // A key step away and back: added and removed again (discrete steps, no drag).
  await host.press("T");
  await host.settle();
  assert.equal(eq(host).contrast, 2 ** 0.01);
  await host.press("R");
  await host.settle();
  assert.equal(node(host, "eq"), undefined);
  await stop();
});

test("settings restored at startup with a non-default colour install the eq node; saved defaults do not", async () => {
  await withData(async (data) => {
    await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, picture: { contrast: 30 } }));
    let { host, window, stop } = await playing(data);
    await host.settle();
    assert.equal(window.props.contrast, 30);
    assert.deepEqual(chain(host), [autoDeinterlace, { kind: "eq", brightness: 0, contrast: 2 ** 0.3, saturation: 1, gamma: 1 }], "restored: present");
    await host.press("Q");
    await stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.picture.contrast, 0, "the reset is saved");
    ({ host, window, stop } = await playing(data));
    await host.settle();
    assert.equal(window.props.contrast, 0);
    assert.deepEqual(kinds(host), ["deinterlace"], "saved defaults: no eq node");
    await stop();
  });
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
  assert.deepEqual(host.osd, ["Sharpen: on", "3D denoise: on", "Temporal denoise: on", "Deblock: on", "Edge sharpen: on", "Deband: on", "Gradual denoise: on"]);
  assert.deepEqual(kinds(host), ["deinterlace", "denoise", "denoise", "denoise", "deblock", "deband", "sharpen", "sharpen"]);
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
  assert.equal(host.osd.at(-1), "Sharpen: off");
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
  const { host, window, stop } = await playing(undefined, zh);
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

test("E10: Ctrl+I cycles 自动 → 总是使用 → 不使用; Auto is the player's deinterlacer, Output applies to Always", async () => {
  const { host, window, stop } = await playing();
  assert.deepEqual(chain(host), [autoDeinterlace], "defaults: the auto node only, no bwdif");
  assert.deepEqual(applied(host), [autoDeinterlace], "accepted by the host");
  await host.press("Ctrl+I");
  await host.settle();
  assert.equal(host.osd.at(-1), "Deinterlace: always");
  assert.deepEqual(chain(host), [{ kind: "deinterlace", when: "all", output: "frame" }]);
  assert.equal(host.command("video.deinterlace.on").state.checked, true);
  assert.equal(host.command("video.deinterlace.output.field").state.enabled, true, "Output applies to Always");
  assert.equal(host.command("video.deinterlace.output.field").state.reason, null);
  await host.command("video.deinterlace.output.field").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Deinterlace output: by field (double rate)");
  assert.deepEqual(chain(host), [{ kind: "deinterlace", when: "all", output: "field" }]);
  assert.equal(window.props["output-index"], 1);
  await host.press("Ctrl+I");
  await host.settle();
  assert.equal(host.osd.at(-1), "Deinterlace: never");
  assert.deepEqual(chain(host), [], "Never: an empty chain");
  assert.equal(applied(host), undefined, "nothing applied");
  assert.equal(window.props["deinterlace-index"], 2);
  assert.equal(host.command("video.deinterlace.output.frame").state.enabled, true);
  await host.press("Ctrl+I");
  await host.settle();
  assert.equal(host.osd.at(-1), "Deinterlace: auto");
  assert.deepEqual(chain(host), [autoDeinterlace], "back to Auto with By Field stored: still the auto node");
  assert.equal(host.command("video.deinterlace.output.field").state.enabled, false, "greyed again under Auto");
  // Output chosen in the window while Auto is selected is kept for Always and changes nothing now.
  const sent = host.sent("processing.replace").length;
  window.emit("choose", "output", 0);
  await host.settle();
  assert.equal(host.sent("processing.replace").length, sent, "an unchanged chain is not resent");
  assert.deepEqual(chain(host), [autoDeinterlace]);
  assert.equal(host.command("video.deinterlace.output.frame").state.checked, true, "stored for Always");
  window.emit("choose", "deinterlace", 1);
  await host.settle();
  assert.deepEqual(chain(host), [{ kind: "deinterlace", when: "all", output: "frame" }], "Always picks up the stored By Frame");
  window.emit("choose", "deinterlace", 0);
  await host.press("E");
  await host.settle();
  assert.deepEqual(chain(host), [autoDeinterlace, { ...neutralEq, brightness: 0.01 }], "auto and a colour node");
  assert.deepEqual(applied(host), chain(host), "applied together");
  assert.equal(chain(host)[0]!.kind, "deinterlace", "always first");
  await stop();
});

test("E10: deinterlace settings restored at startup: Auto with any Output installs the auto node; Always keeps its Output", async () => {
  await withData(async (data) => {
    const cases = [
      [{ mode: "auto", output: "frame" }, [autoDeinterlace], 0, 0],
      [{ mode: "auto", output: "field" }, [autoDeinterlace], 0, 1],
      [{ mode: "on", output: "field" }, [{ kind: "deinterlace", when: "all", output: "field" }], 1, 1],
      [{ mode: "on", output: "frame" }, [{ kind: "deinterlace", when: "all", output: "frame" }], 1, 0],
      [{ mode: "off", output: "field" }, [], 2, 1],
    ] as const;
    for (const [deinterlace, nodes, modeIndex, outputIndex] of cases) {
      await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, deinterlace }));
      const { host, window, stop } = await playing(data);
      await host.settle();
      const what = `${deinterlace.mode} + ${deinterlace.output}`;
      assert.deepEqual(host.sent("processing.replace").at(-1)?.nodes ?? [], nodes, what);
      assert.deepEqual(applied(host) ?? [], nodes, `${what}: applied`);
      assert.equal(window.props["deinterlace-index"], modeIndex, what);
      assert.equal(window.props["output-index"], outputIndex, `${what}: the stored Output is shown`);
      assert.equal(host.command(`video.deinterlace.output.${deinterlace.output}`).state.enabled, deinterlace.mode !== "auto", what);
      await stop();
    }
  });
});

test("E12: pad or crop to a ratio, move the picture in 5% steps, reset the position", async () => {
  const { host, window, stop } = await playing(undefined, zh);
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

test("global settings and the open Advanced section survive a restart; flip and rotation do not", async () => {
  await withData(async (data) => {
    let { host, window, stop } = await playing(data);
    await host.press("O");
    await host.press("Ctrl+R");
    await host.press("Ctrl+P");
    await host.press("Ctrl+I");
    await host.command("video.reframe.pad.4").run();
    window.emit("set-advanced", true);
    window.emit("commit", "edge", 70);
    await stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.version, 1);
    assert.equal(saved.picture.hue, 1);
    assert.equal(saved.advanced, true);
    assert.equal("tab" in saved, false);
    ({ host, window, stop } = await playing(data));
    await host.settle();
    assert.equal(window.props["advanced-open"], true);
    assert.equal(window.props.hue, 1);
    assert.equal(window.props["sharpen-enabled"], true);
    assert.equal(window.props["edge-strength"], 70);
    assert.equal(window.props["flip-v"], false);
    assert.equal(host.command("video.deinterlace.on").state.checked, true);
    assert.equal(host.command("video.reframe.pad.4").state.checked, true);
    assert.deepEqual(kinds(host), ["deinterlace", "reframe", "hue", "sharpen"], "colours at their defaults: no eq");
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
  assert.equal(host.mounted().props.status, "");
  await stop();
});

test("empty media keeps edits and the ready source gets the latest values", async () => {
  const { host, stop } = await start();
  await host.command("video.picture").run();
  const window = host.mounted();
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
  assert.equal(window.props.status, "filter");
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
  assert.equal(window.props.status, "no-picture");
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
  const window = host.mounted();
  await host.command("video.frame-blend").run();
  assert.equal(host.osd.at(-1), "Frame doubling: on");
  assert.equal(host.command("video.frame-blend").state.checked, true);
  assert.equal(window.props["blend-enabled"], true);
  assert.equal(node(host, "frameBlend"), undefined, "not before the rate is read");
  await until(host, blendNode(host, 48), "the 24 fps rate read");
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 48 });
  assert.equal(window.props["blend-enough"], false);
  // Now stats() reports the blended rate; a new 60 fps source is read again without the node.
  setFps(host, 60);
  host.openElsewhere("/sport60.mp4");
  await host.settle();
  assert.equal(node(host, "frameBlend"), undefined);
  await until(host, () => window.props["blend-enough"] === true, "the 60 fps rate read");
  assert.equal(node(host, "frameBlend"), undefined, "60 fps is smooth enough");
  assert.equal(window.props["blend-enough"], true);
  assert.equal(host.command("video.frame-blend").state.checked, true, "the switch stays on");
  // A deinterlace change can change the base rate (Always + By Field doubles it): read again.
  setFps(host, 25);
  await host.command("video.deinterlace.on").run();
  await until(host, blendNode(host, 50), "the rate read again after the deinterlace change");
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 50 });
  window.emit("toggle", "frameBlend", false);
  await host.settle();
  assert.equal(node(host, "frameBlend"), undefined);
  assert.equal(window.props["blend-enough"], false);
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
    await until(host, blendNode(host, 60), "the last try counting an unknown rate as 60");
    await host.command("video.picture").run();
    host.mounted().emit("toggle-effect", false);
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
  const seen: unknown[] = [];
  for (let i = 0; i < 7; i++) {
    display.aspect = nextAspect(display);
    seen.push(aspectMessage(display).args[0] ?? aspectMessage(display).text);
  }
  assert.deepEqual(seen, ["4:3", "16:9", "16:10", "1.85:1", "2.35:1", "Aspect ratio: Default", "4:3"]);
  assert.deepEqual(aspectMessage({ ...display, aspect: 16 / 9 }), { text: "Aspect ratio: {}", args: ["16:9"] });
  display.customAspect = "21:9";
  display.aspect = 2.35;
  display.aspect = nextAspect(display);
  assert.deepEqual(aspectMessage(display), { text: "Aspect ratio: Custom - {}", args: ["21:9"] });
  assert.equal(aspectIndex(display), 6);
  assert.equal(nextAspect(display), null);
  const parsed = parse({ version: 1, view: { aspect: 2.1, customAspect: "abc", fit: "fill" } });
  assert.deepEqual(parsed.display, { aspect: 2.1, customAspect: "2.1", fit: "fill" }, "a lone ratio becomes the custom text");
  assert.deepEqual(parse({ version: 1, view: { aspect: 9, fit: "zoom" } }).display, { aspect: null, customAspect: "", fit: "keep" });
});

test("E11: 比例 menu and Shift+F6 set the display layer with the OSD wording; the chain is untouched", async () => {
  const { host, window, stop } = await playing(undefined, zh);
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
    let { host, stop } = await playing(data, zh);
    await host.command("video.aspect.edit").run();
    const dialog = host.window("aspect");
    assert.equal(dialog.props.value, "");
    dialog.emit("edited", "abc");
    await host.settle();
    assert.equal(dialog.props.valid, false);
    assert.equal(dialog.props.invalid, true, "the format hint shows");
    dialog.emit("confirm", "abc");
    await host.settle();
    assert.ok(host.windows.has("aspect"), "an invalid ratio keeps the window open");
    dialog.emit("edited", "21:9");
    await host.settle();
    assert.equal(dialog.props.valid, true);
    assert.equal(dialog.props.invalid, false);
    dialog.emit("confirm", "21:9");
    await host.settle();
    assert.equal(host.windows.has("aspect"), false);
    assert.equal(host.osd.at(-1), "宽高比：自定义 - 21:9");
    assert.equal(host.videoView.values.aspect, 2.3333);
    const custom = host.command("video.aspect.custom");
    assert.deepEqual(custom.declaration.title, { $vestraText: "Custom ({})", args: ["21:9"] });
    assert.equal(custom.display.title, "自定义（21:9）");
    assert.deepEqual([custom.state.checked, custom.state.enabled, custom.state.reason], [true, true, null]);
    // The window words its 自定义（21:9） choice from the ratio.
    assert.equal(host.mounted().props["custom-ratio"], "21:9");
    assert.equal(host.mounted().props["aspect-index"], 6);
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
  const window = host.mounted();
  assert.deepEqual(host.osd, []);
  assert.deepEqual([window.props.status, window.props.detail, window.props.conflict], ["view", "", true]);
  assert.equal(window.props.failed, true);
  await host.command("video.aspect.0").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Display unavailable: another plugin controls the display");
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
  assert.deepEqual(host.osd, ["Framing: larger (105%)", "Framing: larger (110%)"]);
  assert.equal(host.videoView.values.zoom, 1.1025);
  assert.equal(window.props["zoom-percent"], 110);
  await host.press("Numpad1");
  assert.equal(host.osd.at(-1), "Framing: smaller (105%)");
  for (let i = 0; i < 3; i++) await host.press("Alt+ArrowLeft");
  assert.equal(host.osd.at(-1), "Framing: move left (-3%, 0%)");
  await host.press("Ctrl+Numpad8");
  await host.press("Alt+ArrowDown");
  await host.press("Alt+ArrowDown");
  await host.press("Ctrl+Numpad6");
  await host.settle();
  assert.equal(host.osd.at(-1), "Framing: move right (-2%, 1%)");
  assert.deepEqual([host.videoView.values.panX, host.videoView.values.panY], [-0.02, 0.01]);
  await host.press("Ctrl+Numpad5");
  await host.settle();
  assert.equal(host.osd.at(-1), "Framing: centered");
  assert.deepEqual([host.videoView.values.zoom, host.videoView.values.panX, host.videoView.values.panY], [1.05, 0, 0]);
  for (let i = 0; i < 40; i++) await host.press("Numpad9");
  await host.settle();
  assert.equal(host.videoView.values.zoom, 4);
  assert.equal(host.osd.at(-1), "Framing: larger (400%)", "the edge keeps the value and still reports it");
  await host.press("Numpad5");
  await host.settle();
  assert.equal(host.osd.at(-1), "Framing: reset");
  assert.equal(host.videoView.values.zoom, 1);
  await assert.rejects(host.press("5"), "the main keyboard's 5 is not 帧位复位");
  await stop();
});

test("E13: the panel's zoom, pan and crop buttons run their commands; a new source starts unzoomed, silently", async () => {
  const { host, window, stop } = await playing();
  window.emit("run", "video.frame.zoom-in");
  await until(host, () => window.props["zoom-percent"] === 105, "the zoom shown");
  assert.equal(host.videoView.values.zoom, 1.05);
  window.emit("run", "video.frame.right");
  await until(host, () => window.props["pan-x"] === 1, "the pan shown");
  assert.equal(host.osd.at(-1), "Framing: move right (1%, 0%)", "as the key");
  window.emit("run", "video.frame.centre");
  await until(host, () => window.props["pan-x"] === 0, "centred");
  window.emit("run", "video.frame.reset");
  await until(host, () => window.props["zoom-percent"] === 100, "reset");
  window.emit("run", "video.reframe.right");
  await until(host, () => host.osd.at(-1) === "Crop position (55%, 50%)", "the crop moved");
  window.emit("run", "video.reframe.reset");
  await until(host, () => host.osd.at(-1) === "Crop position (50%, 50%)", "the crop centred");
  window.emit("run", "no.such.command");
  await host.settle();
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

const SOFTWARE = "Needs GPU rendering (currently software rendering)";
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
    let { stop } = await start(host, data, zh);
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
    const window = host.mounted();
    await until(host, () => window.props["jump-serial"] === 1, "a jump");
    assert.equal(window.props["jump-section"], 3, "HDR Settings… shows the Advanced section");
    await until(host, () => window.props["advanced-open"] === true, "Advanced open");
    await until(host, () => host.layout.adjust?.preferred === true, "the panel shown");
    assert.equal(window.props["tone-index"], 2);
    assert.deepEqual([window.props["peak-nits"], window.props["not-hdr"]], [203, true], "the window words 目标亮度 203 尼特 · 当前片源不是 HDR");
    assert.equal(window.props["render-ready"], true);
    window.emit("adjust", "targetPeak", 296.4);
    await host.settle();
    assert.equal(host.videoRender.values.targetPeak, 300, "follows the drag in steps of 10");
    window.emit("reset-field", "targetPeak");
    await until(host, () => host.videoRender.values.targetPeak === 203, "the double-click default");
    window.emit("toggle", "peakDetect", true);
    await until(host, () => host.videoRender.values.peakDetect === true, "peak detection from the panel");
    window.emit("toggle", "peakDetect", false);
    await until(host, () => host.videoRender.values.peakDetect === false, "and off again");
    window.emit("adjust", "targetPeak", 296.4);
    window.emit("choose", "toneMapping", 1);
    await host.settle();
    assert.equal(host.videoRender.values.toneMapping, "off");
    assert.equal(host.osd.at(-1), "动态峰值检测: 关", "window edits give no on-screen text");
    // An HDR10 source: no note.
    setTransfer(host, "pq");
    host.openElsewhere("/hdr10.mkv");
    await host.settle();
    assert.deepEqual([window.props["peak-nits"], window.props["not-hdr"]], [300, false]);
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
  const window = host.mounted();
  assert.equal(window.props["render-ready"], false);
  assert.equal(window.props.status, "", "not a failure in the status line");
  assert.deepEqual(host.osd, []);
  // a disabled command's key shows its reason on the OSD and runs nothing (SDK 1.5.12, as the player)
  const checked = host.command("video.shader.use").state.checked;
  assert.equal(await host.press("Shift+P"), "reason");
  assert.deepEqual(host.osd, [SOFTWARE]);
  assert.equal(host.command("video.shader.use").state.checked, checked, "the shaders were not switched");
  await stop();
});

/** What `run` writes to the plugin log (console.error), as text. */
async function logged(run: () => Promise<void>): Promise<string[]> {
  const lines: string[] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => void lines.push(args.map((a) => (a instanceof Error ? `${(a as Error & { code?: string }).code}: ${a.message}` : String(a))).join(" "));
  try {
    await run();
  } finally {
    console.error = original;
  }
  return lines;
}
const SAVED_RENDER = { toneMapping: "hable", targetPeak: 300, peakDetect: false, shadersEnabled: true, shaders: ["invert.glsl"], interpolation: true } as const;

test("software rendering: nothing in the log, nothing sent after the first refusal; the saved options are kept for GPU rendering", async () => {
  await withData(async (data) => {
    await mkdir(join(data, "shaders"));
    await writeFile(join(data, "shaders", "invert.glsl"), INVERT);
    await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, render: SAVED_RENDER }));
    const host = new TestHost();
    host.videoRender.gpu = false;
    host.openElsewhere("/movie.mp4");
    let sentAtStart = 0;
    const lines = await logged(async () => {
      const { stop } = await start(host, data);
      await host.settle();
      sentAtStart = host.sent("video.render.set").length;
      assert.equal(host.command("video.interpolation").state.enabled, false);
      assert.equal(host.command("video.interpolation").state.reason, SOFTWARE);
      assert.equal(host.command("video.hdr.hable").state.checked, true, "the saved choice still shows");
      // the window's 目标亮度 control is greyed; a stray edit is saved but not sent
      await host.command("video.picture").run();
      host.mounted().emit("adjust", "targetPeak", 250);
      await host.settle();
      await stop();
    });
    assert.deepEqual(lines, [], "software rendering is a state, not an error");
    assert.equal(host.sent("video.render.set").length, sentAtStart, "once refused, nothing more is sent");
    assert.deepEqual(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).render, { ...SAVED_RENDER, targetPeak: 250 });
    // The same profile on GPU rendering (macOS): everything applies.
    const gpu = new TestHost();
    gpu.openElsewhere("/movie.mp4");
    const { stop } = await start(gpu, data);
    await gpu.settle();
    assert.deepEqual(gpu.videoRender.values, {
      toneMapping: "hable",
      targetPeak: 250,
      peakDetect: false,
      shaders: [join(data, "shaders", "invert.glsl")],
      interpolation: true,
    });
    assert.equal(gpu.command("video.interpolation").state.enabled, true);
    await stop();
  });
});

test("GPU rendering: render failures other than software rendering are still logged", async () => {
  const host = new TestHost();
  host.setVideoRenderOwner("other.plugin");
  host.openElsewhere("/movie.mp4");
  const lines = await logged(async () => {
    const { stop } = await start(host);
    await host.settle();
    assert.equal(host.command("video.interpolation").state.enabled, true, "a conflict is not software rendering");
    await stop();
  });
  assert.ok(lines.length > 0 && lines.every((line) => line.includes("conflict")), lines.join("\n"));
  // A shader that does not compile (pushed by the player): logged as before.
  const { host: gpu, stop } = await playing();
  await gpu.settle();
  const compile = await logged(async () => {
    gpu.failVideoRender("failed", "shader compile failed: ERROR: 0:2: Use of undeclared identifier 'nope'");
    await gpu.settle();
  });
  assert.equal(gpu.command("video.interpolation").state.enabled, true);
  assert.ok(compile.length > 0 && compile.every((line) => line.startsWith("[vestra.video] failed: shader compile failed")), compile.join("\n"));
  await stop();
});

test("software rendering after start: the player's push disables the items", async () => {
  const lines = await logged(async () => {
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
  assert.deepEqual(lines, [], "the fallback to software rendering is not an error either");
});

test("E17: 添加… copies the file into the shader folder; Shift+P sends absolute paths; the list is edited in the window", async () => {
  await withData(async (data) => {
    const desk = join(data, "desk");
    await mkdir(desk);
    await writeFile(join(desk, "invert.glsl"), INVERT);
    await writeFile(join(desk, "notes.txt"), "x");
    let host = new TestHost();
    host.openElsewhere("/movie.mp4");
    let { stop } = await start(host, data, zh);
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
    await until(host, () => same(list.props.names, ["invert.glsl"]), "the copied shader in the list");
    const folder = join(data, "shaders");
    assert.equal(await readFile(join(folder, "invert.glsl"), "utf8"), INVERT, "copied into the folder");
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "invert.glsl")]);
    assert.deepEqual(list.props.names, ["invert.glsl"]);
    assert.deepEqual([list.props.count, list.props.max, list.props.missing], [1, 16, [false]], "已启用 1/16（按顺序执行）");
    await rm(join(desk, "invert.glsl"));
    // Files dropped into the folder show below; double-click adds one.
    await writeFile(join(folder, "b.glsl"), INVERT);
    await writeFile(join(folder, "a.hook"), INVERT);
    await host.command("video.shader.list").run();
    await until(host, () => same(list.props.available, ["a.hook", "b.glsl"]), "the folder scan");
    list.emit("pick", 1);
    await until(host, () => same(list.props.available, ["a.hook"]), "the rescan after a pick");
    assert.deepEqual(list.props.names, ["invert.glsl", "b.glsl"]);
    assert.equal(list.props.selected, 1);
    list.emit("up");
    await host.settle();
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "b.glsl"), join(folder, "invert.glsl")], "in list order");
    await host.press("Shift+P");
    await host.settle();
    assert.equal(host.osd.at(-1), "像素着色: 关");
    assert.deepEqual(host.videoRender.values.shaders, [], "off sends no shaders");
    assert.deepEqual(list.props.names, ["b.glsl", "invert.glsl"], "and keeps the list");
    assert.deepEqual([list.props["render-ready"], list.props["shaders-enabled"]], [true, false], "「使用着色器」未打开");
    await host.press("Shift+P");
    assert.equal(host.osd.at(-1), "像素着色: 开 (2)");
    list.emit("select", 1);
    list.emit("remove");
    await until(host, () => same(list.props.available, ["a.hook", "invert.glsl"]), "the rescan after a remove");
    assert.deepEqual(list.props.names, ["b.glsl"]);
    assert.deepEqual(list.props.available, ["a.hook", "invert.glsl"], "removed from the list, not from the folder");
    // 重新载入: a file deleted outside the player leaves the list with a note.
    await rm(join(folder, "b.glsl"));
    await host.command("video.shader.list").run();
    await host.settle();
    assert.deepEqual([list.props.names, list.props.missing], [["b.glsl"], [true]], "shown as b.glsl（文件不存在）");
    await host.command("video.shader.reload").run();
    await until(host, () => host.osd.at(-1) === "已移除不存在的着色器：b.glsl", "the reload scan");
    assert.deepEqual(list.props.names, []);
    await until(host, () => same(list.props.available, ["a.hook", "invert.glsl"]), "the rescan after the reload");
    list.emit("pick", 1);
    await until(host, () => same(list.props.names, ["invert.glsl"]), "the pick after the reload");
    await host.command("video.shader.folder").run();
    await until(host, () => host.system.opened.length > 0, "the folder created and opened");
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
    const { stop } = await start(host, data, zh);
    await host.command("video.picture").run();
    await host.settle();
    assert.deepEqual(host.videoRender.values.shaders, [join(folder, "broken.glsl")]);
    host.failVideoRender("failed", "shader compile failed: ERROR: 0:2: Use of undeclared identifier 'nope'");
    await host.settle();
    const window = host.mounted();
    assert.equal(window.props.status, "shader");
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
  const { stop } = await start(host, undefined, zh);
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
  await until(host, blendNode(host, 48), "the 24 fps rate read");
  assert.deepEqual(chain(host).at(-1), { kind: "frameBlend", fps: 48 });
  await host.command("video.picture").run();
  const window = host.mounted();
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
  const { stop } = await start(host, undefined, zh);
  await host.command("video.picture").run();
  await host.settle();
  const window = host.mounted();
  assert.deepEqual(host.osd, []);
  assert.deepEqual([window.props.status, window.props.detail, window.props.conflict], ["render-conflict", "", true]);
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

// ---------------------------------------------------------------- GPU Rendering (Experimental): the player's renderer choice
const gpuItem = (host: TestHost) => host.command("video.gpu-rendering");
const gpuTitle = (host: TestHost) => gpuItem(host).declaration.title;
const restartItem = "video.gpu-rendering.restart";
/** The item's state with the reason as the menu shows it (the player's message is passed as data). */
const shownState = (host: TestHost) => ({ ...gpuItem(host).state, reason: gpuItem(host).display.reason ?? null });

test("GPU rendering: macOS defaults to Auto on the GPU, so the item is checked and enabled, last in the Video menu", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  await host.settle();
  assert.equal(gpuTitle(host), "GPU Rendering (Experimental)");
  assert.equal(gpuItem(host).declaration.order, 460);
  assert.deepEqual(shownState(host), { enabled: true, checked: true, busy: false, reason: null });
  assert.deepEqual(host.sent("rendering.setPreference"), [], "reading the state changes nothing");
  await stop();
});

test("GPU rendering: Windows without a GPU renderer greys the item out with the player's reason", async () => {
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.rendering.startedWith = "software";
  host.rendering.reason = { code: "no-gpu-backend", message: "This player has no GPU renderer on Windows yet" };
  host.rendering.gpu = { available: false, reason: host.rendering.reason };
  const { stop } = await start(host);
  await host.settle();
  assert.equal(gpuTitle(host), "GPU Rendering (Experimental)");
  assert.deepEqual(shownState(host), { enabled: false, checked: false, busy: false, reason: "This player has no GPU renderer on Windows yet" });
  await stop();
});

test("GPU rendering: a denylisted driver greys the item out with the denylist message", async () => {
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.rendering.startedWith = "software";
  const denied = { code: "denylist", message: "Your graphics driver (AMD Radeon 780M, 31.0.14005.21001) is known to stall; update it" };
  host.rendering.reason = denied;
  host.rendering.gpu = { available: false, reason: denied };
  const { stop } = await start(host);
  await host.settle();
  assert.deepEqual(shownState(host), { enabled: false, checked: false, busy: false, reason: denied.message });
  await stop();
});

test("GPU rendering: switching off waits for a restart; Later keeps the choice; switching back clears it", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await gpuItem(host).run();
  await host.settle();
  assert.deepEqual(
    host.sent("rendering.setPreference").map((r) => r.preference),
    ["software"],
  );
  assert.equal(host.rendering.preference, "software", "saved by the player at once");
  assert.equal(gpuTitle(host), "GPU Rendering (Restart to Apply)");
  assert.equal(gpuItem(host).state.checked, false);
  assert.equal(host.confirms.length, 1);
  assert.equal(host.confirms[0]!.message, "Restart Vestra Player now to apply?");
  assert.deepEqual([host.confirms[0]!.confirm, host.confirms[0]!.cancel], ["Restart Now", "Later"]);
  assert.equal(host.rendering.restarts, 0, "Later does not restart");
  assert.equal(host.osd.at(-1), "GPU rendering: off (applies at the next start)");
  assert.equal(host.command(restartItem).declaration.title, "Restart Player to Apply", "the restart stays offered");
  assert.equal(host.command(restartItem).declaration.order, 460);
  // Back to the mode this run started with: nothing waits, no dialog.
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.rendering.preference, "gpu");
  assert.equal(gpuTitle(host), "GPU Rendering (Experimental)");
  assert.equal(gpuItem(host).state.checked, true);
  assert.equal(host.confirms.length, 1, "no restart is offered when nothing would change");
  assert.equal(host.commands.has(restartItem), false, "the restart item goes with the pending choice");
  assert.equal(host.osd.at(-1), "GPU rendering: on");
  await stop();
});

test("GPU rendering: switching on from software rendering offers the restart, and Restart Now restarts", async () => {
  // A player that can draw on the GPU but started in software mode by the user's choice.
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.rendering.startedWith = "software";
  host.rendering.preference = "software";
  host.rendering.reason = { code: "preference", message: "You chose software rendering" };
  const { stop } = await start(host);
  await host.settle();
  assert.deepEqual(shownState(host), { enabled: true, checked: false, busy: false, reason: null });
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.rendering.preference, "gpu");
  assert.equal(gpuTitle(host), "GPU Rendering (Restart to Apply)");
  assert.equal(gpuItem(host).state.checked, true);
  assert.equal(host.confirms.length, 1);
  assert.equal(host.rendering.restarts, 1);
  await stop();
});

test("GPU rendering: a refused choice says why and greys the item out; a new source reads the state again; a failed save says so", async () => {
  const host = new TestHost();
  host.rendering.preference = "software";
  host.rendering.startedWith = "software";
  host.videoRender.gpu = false;
  const { stop } = await start(host);
  await host.settle();
  assert.equal(gpuItem(host).state.enabled, true);
  // The GPU is ruled out after the item was built (as the player would after a crash guard).
  const crashed = { code: "previous-crash", message: "GPU rendering was turned off after a crash" };
  host.rendering.gpu = { available: false, reason: crashed };
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.osd.at(-1), "GPU rendering unavailable: GPU rendering was turned off after a crash");
  assert.equal(host.rendering.preference, "software", "the old choice stays");
  assert.deepEqual(shownState(host), { enabled: false, checked: false, busy: false, reason: crashed.message });
  assert.deepEqual(host.confirms, []);
  // Available again; a new source reads the state again.
  host.rendering.gpu = { available: true, reason: null };
  host.openElsewhere("/movie.mp4");
  await until(host, () => gpuItem(host).state.enabled, "the item enabled again");
  // The settings file cannot be written.
  host.handle("rendering.setPreference", () => {
    throw new HostError("failed", "disk full");
  });
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Could not save the rendering choice: disk full");
  assert.equal(gpuTitle(host), "GPU Rendering (Experimental)");
  assert.deepEqual(host.confirms, []);
  await stop();
});

test("GPU rendering: a restart the player cannot start is said; the choice stays", async () => {
  const host = new TestHost();
  host.handle("rendering.restart", () => {
    throw new HostError("failed", "could not start the new player");
  });
  const { stop } = await start(host);
  host.dialogAnswers.push({ confirmed: true, checked: false });
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.rendering.preference, "software");
  assert.equal(host.osd.at(-1), "Could not restart the player: could not start the new player");
  await stop();
});

test("GPU rendering: Simplified Chinese titles, dialog and on-screen text", async () => {
  const host = new TestHost();
  const { stop } = await start(host, undefined, zh);
  assert.equal(gpuItem(host).display.title, "GPU 渲染（实验性）");
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await gpuItem(host).run();
  await host.settle();
  assert.equal(gpuItem(host).display.title, "GPU 渲染（重启后生效）");
  assert.equal(host.confirmTexts.at(-1)!.message, "现在重启 Vestra Player 以应用更改？");
  assert.equal(host.confirmTexts.at(-1)!.cancel, "稍后");
  assert.equal(host.osd.at(-1), "GPU 渲染: 关（下次启动时生效）");
  await stop();
});

test("GPU rendering: Restart Player to Apply restarts later; a failed restart is said", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  host.dialogAnswers.push({ confirmed: false, checked: false });
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.rendering.restarts, 0);
  host.handle("rendering.restart", () => {
    throw new HostError("failed", "could not start the new player");
  });
  await host.command(restartItem).run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Could not restart the player: could not start the new player");
  host.reset("rendering.restart");
  await host.command(restartItem).run();
  await host.settle();
  assert.equal(host.rendering.restarts, 1);
  assert.equal(host.rendering.preference, "software", "the choice is not touched by the restart");
  await stop();
});

test("GPU rendering: a checked item stays usable when the GPU is ruled out, so the user can leave it", async () => {
  // The user chose the GPU; then GPU rendering crashed at a start, so this run draws in software.
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.rendering.preference = "gpu";
  host.rendering.startedWith = "software";
  const crashed = { code: "previous-crash", message: "GPU rendering was turned off after a crash" };
  host.rendering.reason = crashed;
  host.rendering.gpu = { available: false, reason: crashed };
  const { stop } = await start(host);
  await host.settle();
  assert.deepEqual(shownState(host), { enabled: true, checked: true, busy: false, reason: null }, "no reason while it can be used");
  await gpuItem(host).run();
  await host.settle();
  assert.deepEqual(
    host.sent("rendering.setPreference").map((r) => r.preference),
    ["software"],
  );
  assert.equal(host.rendering.preference, "software");
  assert.equal(host.osd.at(-1), "GPU rendering: off");
  assert.deepEqual(host.confirms, [], "this run already draws in software: nothing to restart");
  assert.deepEqual(shownState(host), { enabled: false, checked: false, busy: false, reason: crashed.message }, "now unchecked, it greys out with the reason");
  await stop();
});

test("GPU rendering: chosen but still ruled out at the next start (VESTRA_RENDER), the on-screen text says why", async () => {
  const host = new TestHost();
  host.rendering.preference = "software";
  host.rendering.startedWith = "software";
  host.videoRender.gpu = false;
  const env = { code: "env", message: "VESTRA_RENDER=cpu asks for software rendering" };
  host.handle("rendering.setPreference", () => ({
    mode: "software",
    reason: env,
    preference: "gpu",
    nextMode: "software",
    nextReason: env,
    restartPending: false,
    gpu: { available: true, reason: null },
  }));
  const { stop } = await start(host);
  await gpuItem(host).run();
  await host.settle();
  assert.equal(host.osd.at(-1), "GPU rendering chosen; the next start still uses software rendering: VESTRA_RENDER=cpu asks for software rendering");
  assert.deepEqual(host.confirms, []);
  await stop();
});

test("GPU rendering: while a switch is under way the item is busy with a reason and a second pick does nothing", async () => {
  const host = new TestHost();
  const { stop } = await start(host);
  let release!: () => void;
  const held = new Promise<void>((done) => (release = done));
  let answered = 0;
  host.handle("dialogs.confirm", async () => {
    answered++;
    await held;
    return { confirmed: false, checked: false };
  });
  // `run()` and `settle()` wait for an idle host, which the open question holds: poll by hand.
  const first = gpuItem(host).run();
  for (let i = 0; i < 1200 && !(answered === 1 && gpuItem(host).state.busy); i++) await new Promise((done) => setTimeout(done, 5));
  assert.deepEqual(shownState(host), { enabled: true, checked: false, busy: true, reason: "Switching…" });
  // The player does not run a busy command; the plugin also ignores a pick that slips through.
  await assert.rejects(Promise.resolve().then(() => gpuItem(host).run()), { code: "command" }, "a busy item does not run");
  release();
  await first;
  await host.settle();
  assert.deepEqual(
    host.sent("rendering.setPreference").map((r) => r.preference),
    ["software"],
    "one switch only",
  );
  assert.equal(answered, 1);
  assert.equal(gpuItem(host).state.busy, false);
  assert.equal(shownState(host).reason, null);
  await stop();
});

// ---------------------------------------------------------------- the Picture panel (adjust column)

type Gpu = { shown: boolean; checked: boolean; enabled: boolean; busy: boolean; pending: boolean; reason: string };
const gpuOf = (host: TestHost) => host.mounted().props.gpu as Gpu;
const keysOf = (host: TestHost) => host.mounted().props.keys as Record<string, string>;

test("panel key caps show each command's current first key and follow the user's changes", async () => {
  const host = new TestHost();
  host.systemInfo.os = "windows";
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  try {
    await eventually(() => keysOf(host), {
      brightnessdown: "W",
      brightnessup: "E",
      contrastdown: "R",
      contrastup: "T",
      saturationdown: "Y",
      saturationup: "U",
      huedown: "I",
      hueup: "O",
      sharpen: "Ctrl+R",
      edge: "",
      blur: "Ctrl+B",
      deblock: "Ctrl+H",
      deband: "",
      denoise: "Ctrl+N",
      doubling: "",
      interpolation: "",
      aspect: "Shift+F6",
      rotate: "Alt+K",
      flipv: "Ctrl+P",
      fliph: "Ctrl+Z",
      zoomout: "Alt+-",
      zoomin: "Alt++",
      panleft: "Alt+←",
      panup: "Alt+↑",
      pandown: "Alt+↓",
      panright: "Alt+→",
      centre: "Ctrl+Num5",
      framing: "Alt+Backspace",
      deinterlace: "Ctrl+I",
      shaders: "Shift+P",
      gpu: "",
      reset: "Q",
      resetpanel: "",
    });
    // The denoise cap is the shown mode's: Temporal has Ctrl+M, Gradual none.
    host.mounted().emit("choose", "denoiseMode", 2);
    await eventually(() => keysOf(host).denoise, "Ctrl+M");
    host.mounted().emit("choose", "denoiseMode", 1);
    await eventually(() => keysOf(host).denoise, "");
    host.rebind("video.brightness.down", ["Ctrl+Shift+W"]);
    await eventually(() => keysOf(host).brightnessdown, "Ctrl+Shift+W");
    host.rebind("video.reset", []);
    await eventually(() => keysOf(host).reset, "", "a cleared key shows no cap");
    host.rebind("video.edge", ["Ctrl+Alt+E"]);
    await eventually(() => keysOf(host).edge, "Ctrl+Alt+E", "a key the user adds shows too");
  } finally {
    await stop();
  }
});

test("panel: a slider's double-click resets that slider only; Q resets colours and effects only", async () => {
  const { host, window, stop } = await playing();
  try {
    window.emit("commit", "saturation", -56);
    window.emit("commit", "contrast", 4);
    await until(host, () => window.props.saturation === -56 && window.props.contrast === 4, "the colours set");
    window.emit("reset-field", "saturation");
    await until(host, () => window.props.saturation === 0, "saturation back to 0");
    await eventually(() => window.props.contrast, 4, "contrast kept");
    window.emit("toggle", "sharpen", true);
    await until(host, () => window.props["sharpen-enabled"] === true, "sharpen on");
    await host.command("video.flip.vertical").run();
    await until(host, () => window.props["flip-v"] === true, "flipped");
    await host.press("Q");
    await until(host, () => window.props.contrast === 0, "Q");
    await eventually(() => window.props["sharpen-enabled"], false, "image processing off");
    await eventually(() => window.props["flip-v"], true, "Q leaves the geometry");
    await eventually(() => host.osd.at(-1), "Picture reset; image processing off");
  } finally {
    await stop();
  }
});

test("panel: the GPU Rendering switch shows the player's state and switches like the command", async () => {
  const host = new TestHost();
  host.videoRender.gpu = false;
  host.rendering.startedWith = "software";
  host.rendering.reason = { code: "no-gpu-backend", message: "This player has no GPU renderer on Windows yet" };
  host.rendering.gpu = { available: false, reason: host.rendering.reason };
  const { stop } = await start(host);
  try {
    await eventually(() => gpuOf(host), { shown: true, checked: false, enabled: false, busy: false, pending: false, reason: "This player has no GPU renderer on Windows yet" });
    await eventually(() => host.mounted().props["render-ready"], false, "HDR, shaders and interpolation greyed");
  } finally {
    await stop();
  }
  const mac = new TestHost();
  const second = await start(mac);
  try {
    await eventually(() => gpuOf(mac), { shown: true, checked: true, enabled: true, busy: false, pending: false, reason: "" });
    mac.dialogAnswers.push({ confirmed: false, checked: false });
    mac.mounted().emit("run", "video.gpu-rendering");
    await until(mac, () => gpuOf(mac).pending, "the restart pending");
    await eventually(() => gpuOf(mac), { shown: true, checked: false, enabled: true, busy: false, pending: true, reason: "" });
    await eventually(() => mac.rendering.preference, "software");
    mac.mounted().emit("run", "video.gpu-rendering.restart");
    await until(mac, () => mac.rendering.restarts === 1, "Restart");
  } finally {
    await second.stop();
  }
});

test("panel: the Advanced section opens and closes, and the jump bar's request reaches the panel", async () => {
  await withData(async (data) => {
    const { host, window, stop } = await playing(data);
    try {
      await eventually(() => window.props["advanced-open"], false, "closed by default");
      window.emit("set-advanced", true);
      await until(host, () => window.props["advanced-open"] === true, "open");
      window.emit("set-advanced", false);
      await until(host, () => window.props["advanced-open"] === false, "closed");
      await host.command("video.hdr.settings").run();
      await until(host, () => window.props["jump-serial"] === 1, "one jump");
      await eventually(() => window.props["jump-section"], 3);
      await eventually(() => window.props["advanced-open"], true, "HDR Settings… opens Advanced");
      await host.command("video.hdr.settings").run();
      await until(host, () => window.props["jump-serial"] === 2, "every request jumps, also to the same section");
    } finally {
      await stop();
    }
  });
});

test("panel: Reset All (video.resetPanel) resets Colour, Effects and Geometry, not the Advanced preferences", async () => {
  const { host, window, stop } = await playing();
  try {
    window.emit("commit", "contrast", 30);
    window.emit("toggle", "sharpen", true);
    window.emit("choose", "fit", 2);
    window.emit("choose", "aspect", 2);
    window.emit("choose", "reframe", 2);
    window.emit("choose", "rotate", 1);
    window.emit("toggle", "flipH", true);
    window.emit("choose", "deinterlace", 1);
    window.emit("choose", "toneMapping", 3);
    window.emit("run", "video.frame.zoom-in");
    window.emit("run", "video.frame.right");
    window.emit("run", "video.reframe.right");
    await until(host, () => window.props["pan-x"] === 1 && window.props["zoom-percent"] === 105, "everything set");
    assert.equal(host.command("video.resetPanel").declaration.home, "video.picture");
    assert.deepEqual(host.command("video.resetPanel").declaration.bindings, [], "no default key");
    window.emit("run", "video.resetPanel");
    await until(host, () => host.osd.at(-1) === "Picture panel reset", "the reset said");
    const shown = () => [window.props.contrast, window.props["sharpen-enabled"], window.props["fit-index"], window.props["aspect-index"], window.props["reframe-index"], window.props["rotate-index"], window.props["flip-h"], window.props["zoom-percent"], window.props["pan-x"]];
    await eventually(shown, [0, false, 0, 0, 0, 0, false, 100, 0]);
    await eventually(() => [window.props["deinterlace-index"], window.props["tone-index"]], [1, 3], "the Advanced preferences stay");
    await eventually(() => [host.videoView.values.zoom, host.videoView.values.panX, host.videoView.values.aspect, host.videoView.values.fit], [1, 0, null, "keep"]);
    await eventually(() => host.command("video.reframe.none").state.checked, true);
    await eventually(() => kinds(host).filter((k) => k !== "deinterlace"), [], "no colour, effect, crop or transform node left");
  } finally {
    await stop();
  }
});
