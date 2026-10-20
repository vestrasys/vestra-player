import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

const directory = new URL("..", import.meta.url);

test("Tab toggles the information overlay; off by default; shows the old diagnostic facts", async () => {
  const host = new TestHost();
  host.stats = {
    backend: "libmpv · 软件渲染",
    session: 3,
    framesShown: 120,
    framesDropped: 1,
    video: { codec: "h264", width: 1920, height: 1080, fps: 24 },
    audio: { codec: "aac", sampleRate: 48000, channels: 2 },
  };
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host });
  const command = host.command("info.toggle");
  assert.deepEqual(command.declaration, {
    title: "播放信息",
    path: ["视图", "播放信息"],
    order: 503,
    group: "domain",
    radio: false,
    bindings: ["Tab"],
    focus: null,
    repeat: false,
  });
  assert.equal(command.state.checked, false);
  assert.equal(host.osdPanel, null);
  await host.press("Tab");
  await host.settle();
  assert.equal(command.state.checked, true);
  assert.equal(
    host.osdPanel,
    [
      "/movie.mp4",
      "00:05 / 01:40",
      "视频 h264 1920×1080 24.00 fps",
      "音频 aac 48000 Hz 2 声道",
      "libmpv · 软件渲染 · session 3 · 帧 120 · 丢帧 1",
    ].join("\n"),
  );
  await host.press("Tab");
  await host.settle();
  assert.equal(host.osdPanel, null);
  assert.equal(command.state.checked, false);
  await stop();
});
