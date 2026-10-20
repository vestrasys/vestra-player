import assert from "node:assert/strict";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";
import { defaults, nodesFor } from "../model.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost()) => startPlugin(activate, { directory, host });
const latest = (host: TestHost) => host.sent("processing.replace").at(-1)!;

test("neutral and blur use a single typed ordered chain", () => {
  assert.deepEqual(nodesFor(defaults), [{ kind: "eq", brightness: 0, contrast: 1, saturation: 1, gamma: 1 }]);
  assert.deepEqual(nodesFor({ ...defaults, blurEnabled: true, blurStrength: 4 })[1], { kind: "gblur", sigma: 4, steps: 3 });
  assert.equal(nodesFor({ ...defaults, blurEnabled: true, blurStrength: 0 }).length, 1);
});

test("menu under 视频 follows the allocation; the window opens once; disabling clears only own nodes", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  const declared = [...host.commands.values()].map(({ declaration: d }) => [d.path.join(" ▶ "), d.order, d.bindings.join(",")]);
  assert.deepEqual(declared, [
    ["视频 ▶ 画面调节…", 401, ""],
    ["视频 ▶ 复位", 402, "Q"],
    ["视频 ▶ 亮度 -1%", 403, "W"],
    ["视频 ▶ 亮度 +1%", 404, "E"],
    ["视频 ▶ 对比度 -1%", 405, "R"],
    ["视频 ▶ 对比度 +1%", 406, "T"],
    ["视频 ▶ 饱和度 -1%", 407, "Y"],
    ["视频 ▶ 饱和度 +1%", 408, "U"],
    ["视频 ▶ 高斯模糊", 409, "Ctrl+B"],
  ]);
  assert.ok([...host.commands.values()].every(({ declaration: d }) => d.group === "picture"));
  assert.deepEqual(host.conflicts, []);
  await host.command("video.picture").run();
  await host.command("video.picture").run();
  assert.equal(host.sent("ui.open").length, 1);
  const window = host.window("picture");
  window.emit("adjust", "brightness", 40);
  await host.settle();
  assert.equal((latest(host).nodes as { brightness: number }[])[0]!.brightness, 0.4);
  assert.equal(window.props.status, "");
  assert.equal(window.props.failed, false);
  window.emit("toggle-blur", true);
  await host.settle();
  assert.equal((latest(host).nodes as { kind: string }[])[1]!.kind, "gblur");
  assert.equal(host.command("video.gaussian").state.checked, true);
  window.emit("toggle-effect", false);
  await host.settle();
  assert.deepEqual(latest(host).nodes, []);
  assert.equal(latest(host).resourceId, "picture");
  assert.equal(window.props["effect-enabled"], false);
  assert.ok(host.sent("processing.replace").every((c) => c.expectedGeneration === 1));
  assert.deepEqual(host.osd, [], "window edits give no on-screen text");
  await stop();
});

test("PotPlayer keys step colors by 1% with on-screen feedback; Q resets; Ctrl+B toggles blur", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.press("E");
  await host.press("E");
  await host.press("W");
  await host.press("T");
  await host.press("Y");
  await host.settle();
  assert.deepEqual(host.osd, ["亮度 +1% (+1%)", "亮度 +1% (+2%)", "亮度 -1% (+1%)", "对比度 +1% (+1%)", "饱和度 -1% (-1%)"]);
  const eq = (latest(host).nodes as { brightness: number; contrast: number; saturation: number }[])[0]!;
  assert.equal(eq.brightness, 0.01);
  assert.equal(eq.contrast, 2 ** 0.01);
  assert.equal(eq.saturation, 0.99);
  await host.press("R");
  await host.press("U");
  assert.equal(host.osd.at(-1), "饱和度 +1% (0%)");
  // A dragged fractional value steps from its rounded value; limits hold.
  await host.command("video.picture").run();
  host.window("picture").emit("adjust", "brightness", 99.6);
  await host.settle();
  await host.press("E");
  await host.press("E");
  assert.equal(host.window("picture").props.brightness, 100);
  assert.equal(host.osd.at(-1), "亮度 +1% (+100%)");
  await host.press("Ctrl+B");
  await host.settle();
  assert.equal(host.osd.at(-1), "高斯模糊: 开");
  assert.equal(host.command("video.gaussian").state.checked, true);
  assert.equal(host.window("picture").props["blur-enabled"], true);
  assert.equal((latest(host).nodes as unknown[]).length, 2);
  await host.press("Ctrl+B");
  assert.equal(host.osd.at(-1), "高斯模糊: 关");
  await host.press("Q");
  await host.settle();
  assert.equal(host.osd.at(-1), "画面调节已复位");
  assert.deepEqual(latest(host).nodes, nodesFor(defaults));
  assert.equal(host.window("picture").props.brightness, 0);
  await stop();
});

test("each 默认 button resets only its own row", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  window.emit("adjust", "contrast", 30);
  window.emit("adjust", "gamma", 20);
  window.emit("adjust", "blurStrength", 9);
  window.emit("reset-field", "contrast");
  window.emit("reset-field", "blurStrength");
  window.emit("reset-field", "nonsense");
  await host.settle();
  assert.equal(window.props.contrast, 0);
  assert.equal(window.props.gamma, 20);
  assert.equal(window.props["blur-strength"], defaults.blurStrength);
  assert.equal((latest(host).nodes as { gamma: number }[])[0]!.gamma, 2 ** 0.2);
  await stop();
});

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
  assert.equal((latest(host).nodes as { brightness: number }[])[0]!.brightness, 0.01);
  assert.equal(host.window("picture").props.status, "");
  await stop();
});

test("presets and reset replace the whole picture", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  window.emit("preset", "soft");
  await host.settle();
  assert.equal(window.props.contrast, -8);
  assert.equal(window.props["blur-enabled"], true);
  assert.equal((latest(host).nodes as unknown[]).length, 2);
  await host.command("video.reset").run();
  await host.settle();
  assert.equal(window.props.contrast, 0);
  assert.deepEqual(latest(host).nodes, nodesFor(defaults));
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
  assert.equal((latest(host).nodes as { contrast: number }[])[0]!.contrast, 2 ** 0.35);
  assert.equal(latest(host).expectedGeneration, 1);
  await stop();
});

test("a refused change is shown and retried on request; a lost reply never sticks", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  host.handle("processing.replace", () => {
    throw new HostError("unsupported", "no video stream");
  });
  window.emit("adjust", "gamma", 20);
  await host.settle();
  assert.equal(window.props.status, "画面处理失败");
  assert.equal(window.props.failed, true);
  assert.equal(window.props.detail, "no video stream");
  host.reset("processing.replace");
  window.emit("retry");
  await host.settle();
  assert.equal(window.props.status, "");
  assert.equal(window.props.failed, false);
  // The host applies the change but its reply is lost: the SDK resolves it.
  host.handle("processing.replace", async (params) => {
    host.reset("processing.replace");
    await host.request("processing.replace", params);
    throw new Error("reply lost");
  });
  window.emit("adjust", "gamma", 30);
  await host.settle();
  assert.equal(window.props.failed, false);
  assert.equal((host.processing.resources.get("video:picture")!.nodes as { gamma: number }[])[0]!.gamma, 2 ** 0.3);
  await stop();
});

test("a burst of slider edits keeps one change in flight and ends on the last value", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  const sent = host.sent("processing.replace").length;
  for (let value = 1; value <= 30; value++) window.emit("adjust", "brightness", value);
  await host.settle();
  assert.ok(host.sent("processing.replace").length - sent <= 3);
  assert.equal((latest(host).nodes as { brightness: number }[])[0]!.brightness, 0.3);
  await stop();
});

test("blur strength above the host's 0–10 sigma range is clamped to 10", async () => {
  assert.deepEqual(nodesFor({ ...defaults, blurEnabled: true, blurStrength: 15 })[1], { kind: "gblur", sigma: 10, steps: 3 });
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  await host.command("video.picture").run();
  const window = host.window("picture");
  window.emit("toggle-blur", true);
  window.emit("adjust", "blurStrength", 18);
  await host.settle();
  assert.equal(window.props["blur-strength"], 10);
  assert.deepEqual((latest(host).nodes as unknown[])[1], { kind: "gblur", sigma: 10, steps: 3 });
  assert.equal(window.props.failed, false);
  await stop();
});
