import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { BindingInfo } from "@vestra-player/sdk";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { firstKey } from "../tips.ts";

const directory = new URL("..", import.meta.url);
const start = (host = new TestHost(), dataDirectory?: string) =>
  startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
function withMedia(host = new TestHost()) {
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 100, position: 12 });
  return host;
}
async function until(predicate: () => boolean | Promise<boolean>) {
  const deadline = Date.now() + 2000;
  while (!(await predicate())) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const settingsFile = async (data: string) => {
  try {
    return JSON.parse(await readFile(join(data, "settings.json"), "utf8")) as Record<string, unknown>;
  } catch {
    return {};
  }
};

// ------------------------------------------------------------------ G-07

test("G-07 every bar element has a tooltip naming its key; back/forward explain the right click", async () => {
  const { host, stop } = await start(withMedia());
  const bar = host.mounted();
  // The player announces the table after the plugin's commands are declared.
  host.emit("bindings.changed", null);
  await until(() => bar.props["tip-open"] === "打开文件 (Ctrl+O)");
  // The test host labels a binding with its input name (the player shows ← → PgUp PgDn).
  assert.equal(bar.props["tip-play-pause"], "暂停 (Space)");
  assert.equal(bar.props["tip-stop"], "停止 (F4)");
  assert.equal(bar.props["tip-back"], "后退 5 秒 (ArrowLeft)\n右键：播放上一个 (PageUp)");
  assert.equal(bar.props["tip-forward"], "前进 5 秒 (ArrowRight)\n右键：播放下一个 (PageDown)");
  assert.equal(bar.props["tip-open"], "打开文件 (Ctrl+O)");
  assert.equal(bar.props["tip-time"], "点击切换剩余时间");
  assert.equal(bar.props["tip-mute"], "静音 (M)");
  assert.equal(bar.props["tip-volume"], "音量: 100%");
  // Nobody registered the top-level 「播放列表」 item here: no key.
  assert.equal(bar.props["tip-sidebar"], "播放列表");

  host.setSource({ paused: true, output: { volume: 35, muted: true } });
  await host.settle();
  assert.equal(bar.props["tip-play-pause"], "播放 (Space)");
  assert.equal(bar.props["tip-mute"], "取消静音 (M)");
  assert.equal(bar.props["tip-volume"], "音量: 35%");

  // A rebound key shows at once (the table is read again on bindings.changed).
  host.bindings.overlay.set("host/play-pause", ["K"]);
  host.bindings.overlay.set("host/previous", []);
  host.emit("bindings.changed", null);
  await until(() => bar.props["tip-play-pause"] === "播放 (K)");
  assert.equal(bar.props["tip-back"], "后退 5 秒 (ArrowLeft)\n右键：播放上一个");
  await stop();
});

test("G-07 the key is the first keyboard binding still in force: mouse inputs and shadowed keys are skipped", () => {
  const info = (current: string[], shadowed: string[] = []): BindingInfo => ({
    plugin: "host",
    command: "next",
    title: "下一个",
    path: ["播放", "下一个"],
    scope: "global",
    defaults: current,
    current,
    overridden: false,
    shadowed: shadowed.map((input) => ({ input, by: { plugin: "x", command: "y" } })),
    labels: current.map((input) => input.replace("PageDown", "PgDn")),
  });
  assert.equal(firstKey(info(["Mouse:X2", "PageDown"])), "PgDn");
  assert.equal(firstKey(info(["PageDown", "N"], ["PageDown"])), "N");
  assert.equal(firstKey(info(["Mouse:X2"])), null);
  assert.equal(firstKey(info([])), null);
});

// ------------------------------------------------------------------ G-14

test("G-14 the elapsed/remaining choice is saved and kept after a restart", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-g14-"));
  try {
    const first = await start(withMedia(), data);
    const bar = first.host.mounted();
    assert.equal(bar.props["elapsed-label"], "00:12");
    bar.emit("toggle-time");
    await first.host.settle();
    assert.equal(bar.props["elapsed-label"], "-01:28");
    assert.equal(bar.props["tip-time"], "点击切换已播放时间");
    await until(async () => (await settingsFile(data)).remainingTime === true);
    await first.stop();

    const second = await start(withMedia(), data);
    assert.equal(second.host.mounted().props["elapsed-label"], "-01:28");
    second.host.mounted().emit("toggle-time");
    await second.host.settle();
    assert.equal(second.host.mounted().props["elapsed-label"], "00:12");
    await until(async () => (await settingsFile(data)).remainingTime === false);
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ IA-PB-01

test("IA-PB-01 the hover time is off by default; a saved choice is kept", async () => {
  const data = mkdtempSync(join(tmpdir(), "playback-ia-pb-01-"));
  try {
    const first = await start(new TestHost(), data);
    assert.equal(first.host.command("hover-time").state.checked, false);
    assert.equal(first.host.mounted().props["hover-time"], false);
    await first.stop();

    writeFileSync(join(data, "settings.json"), JSON.stringify({ hoverTime: true, seekSteps: [0.5, 30, 60, 300] }));
    const second = await start(new TestHost(), data);
    assert.equal(second.host.command("hover-time").state.checked, true);
    assert.equal(second.host.mounted().props["hover-time"], true);
    // G-07: the back/forward tooltips follow the first seek tier (B06).
    assert.match(String(second.host.mounted().props["tip-forward"]), /^前进 0\.5 秒 /);
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------ G-18

test("G-18 the playlist button: half lit only when on but too narrow; fullscreen's collapsed pop-out counts as on", async () => {
  const host = withMedia();
  const { stop } = await start(host);
  const bar = host.mounted();
  const show = async (slot: { preferred: boolean; effective: boolean; reason: string | null }) => {
    host.layout.sidebar = { owner: 2, detached: false, ...slot } as never;
    host.emit("window.state", {});
    await host.settle();
    return [bar.props["sidebar-on"], bar.props["sidebar-hidden"], bar.props["tip-sidebar"]];
  };
  assert.deepEqual(await show({ preferred: true, effective: true, reason: null }), [true, false, "播放列表"]);
  assert.deepEqual(await show({ preferred: true, effective: false, reason: "space" }), [
    true,
    true,
    "播放列表（窗口太窄，放不下播放列表）",
  ]);
  assert.deepEqual(await show({ preferred: true, effective: false, reason: "fullscreen" }), [true, false, "播放列表"]);
  assert.deepEqual(await show({ preferred: false, effective: false, reason: null }), [false, false, "播放列表"]);
  await stop();
});
