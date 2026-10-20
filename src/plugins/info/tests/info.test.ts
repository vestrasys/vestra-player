import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate, { fileName } from "../main.ts";

const directory = new URL("..", import.meta.url);
const stats = {
  backend: "libmpv · 软件渲染",
  session: 3,
  framesShown: 120,
  framesDropped: 1,
  video: { codec: "h264", width: 1920, height: 1080, fps: 24 },
  audio: { codec: "aac", sampleRate: 48000, channels: 2 },
};

test("top-level 播放信息 entry in the windows group, Tab toggles the overlay; off by default", async () => {
  const host = new TestHost();
  host.stats = stats;
  host.openElsewhere("/videos/movie.mp4");
  host.setSource({ duration: 100, position: 5 });
  const { stop } = await startPlugin(activate, { directory, host });
  const command = host.command("info.toggle");
  assert.deepEqual(command.declaration, {
    title: "播放信息",
    path: ["播放信息"],
    order: 810,
    group: "windows",
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
      "文件：movie.mp4",
      "时间：00:05 / 01:40",
      "视频：h264 1920×1080",
      "帧率：24.00 fps",
      "音频：aac 48000 Hz 2 声道",
      "后端：libmpv · 软件渲染（会话 3）",
      "已显示帧：120",
      "丢帧：1",
    ].join("\n"),
  );
  await host.press("Tab");
  await host.settle();
  assert.equal(host.osdPanel, null);
  assert.equal(command.state.checked, false);
  await stop();
});

test("refreshes once per second while shown, and stops when hidden", async () => {
  const host = new TestHost();
  host.stats = { ...stats, framesShown: 1 };
  host.openElsewhere("/movie.mp4");
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("info.toggle").run();
  await host.settle();
  assert.match(String(host.osdPanel), /已显示帧：1\n/);
  host.stats = { ...stats, framesShown: 25 };
  await host.settle(1100);
  assert.match(String(host.osdPanel), /已显示帧：25\n/);
  await host.command("info.toggle").run();
  await host.settle();
  host.stats = { ...stats, framesShown: 50 };
  await host.settle(1100);
  assert.equal(host.osdPanel, null);
  await stop();
});

test("without media every fact is a dash", async () => {
  const host = new TestHost();
  host.stats = { backend: "libmpv · 软件渲染", session: 0, framesShown: 0, framesDropped: null, video: null, audio: null };
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("info.toggle").run();
  await host.settle();
  assert.equal(
    host.osdPanel,
    [
      "文件：未打开媒体",
      "时间：--:-- / --:--",
      "视频：- -",
      "帧率：-",
      "音频：- - -",
      "后端：libmpv · 软件渲染（会话 0）",
      "已显示帧：0",
      "丢帧：-",
    ].join("\n"),
  );
  await stop();
});

test("file names come from paths and URLs", () => {
  assert.equal(fileName("C:\\media\\a b.mkv"), "a b.mkv");
  assert.equal(fileName("https://cdn.test/v/%E8%A7%86%E9%A2%91.mp4?x=1"), "视频.mp4");
  assert.equal(fileName("https://www.youtube.com/watch?v=abc"), "watch");
  assert.equal(fileName("https://youtu.be/"), "youtu.be");
});
