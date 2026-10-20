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

test("both menu entrances open one tool; hide keeps the effect; disabling clears only own nodes", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  const { stop } = await start(host);
  assert.deepEqual(
    [...host.commands.values()].map((c) => c.declaration.path),
    [["视频", "画面调节…"], ["视频", "高斯模糊…"]],
  );
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d }) => ("order" in d ? d.order : null)),
    [325, 326],
  );
  await host.command("video.picture").run();
  await host.command("video.gaussian").run();
  assert.equal(host.sent("ui.open").length, 1);
  const window = host.window("picture");
  window.emit("adjust", "brightness", 40);
  await host.settle();
  assert.equal((latest(host).nodes as { brightness: number }[])[0]!.brightness, 0.4);
  assert.equal(window.props.status, "画面设置已接受");
  window.emit("toggle-blur", true);
  await host.settle();
  assert.equal((latest(host).nodes as { kind: string }[])[1]!.kind, "gblur");
  const before = host.sent("processing.replace").length;
  window.emit("hide-tool");
  await host.settle();
  assert.equal(window.visible, false);
  assert.equal(host.sent("processing.replace").length, before);
  window.emit("toggle-effect", false);
  await host.settle();
  assert.deepEqual(latest(host).nodes, []);
  assert.equal(latest(host).resourceId, "picture");
  assert.equal(window.props["effect-enabled"], false);
  assert.ok(host.sent("processing.replace").every((c) => c.expectedGeneration === 1));
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
  window.emit("reset");
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
  assert.equal(window.props.status, "没有媒体；已保留设置");
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
  assert.match(String(window.props.status), /^画面处理失败：/);
  host.reset("processing.replace");
  window.emit("retry");
  await host.settle();
  assert.equal(window.props.status, "画面设置已接受");
  // The host applies the change but its reply is lost: the SDK resolves it.
  host.handle("processing.replace", async (params) => {
    host.reset("processing.replace");
    await host.request("processing.replace", params);
    throw new Error("reply lost");
  });
  window.emit("adjust", "gamma", 30);
  await host.settle();
  assert.equal(window.props.status, "画面设置已接受");
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
