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
const start = (host = new TestHost(), dataDirectory?: string, locale: "en" | "zh-Hans" | "ja" = "zh-Hans") =>
  startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });

/**
 * The tooltips as `ui/bar.slint` words them from the bar's data (the test
 * host does not run Slint), translated with the plugin's catalog.
 */
function tips(host: TestHost, props: Record<string, unknown>) {
  const t = (text: string, args?: (string | number)[]) => host.translate(text, args);
  const withKey = (title: string, key: unknown) => (key === "" ? title : t("{} ({})", [title, String(key)]));
  return {
    playPause: withKey(props.playing ? t("Pause") : t("Play"), props["key-play-pause"]),
    stop: withKey(t("Stop"), props["key-stop"]),
    back: t("{}\nRight-click: {}", [
      withKey(t("Back {} seconds", [String(props["step-text"])]), props["key-back"]),
      withKey(t("Play previous"), props["key-previous"]),
    ]),
    forward: t("{}\nRight-click: {}", [
      withKey(t("Forward {} seconds", [String(props["step-text"])]), props["key-forward"]),
      withKey(t("Play next"), props["key-next"]),
    ]),
    open: withKey(t("Open file"), props["key-open"]),
    time: props["remaining-time"] ? t("Click to show the elapsed time") : t("Click to show the remaining time"),
    mute: withKey(props.muted ? t("Unmute") : t("Mute"), props["key-mute"]),
    volume: t("Volume: {}%", [Number(props["volume-percent"])]),
  };
}

test("G-07 bar.slint words the tooltips from the data this test mirrors", async () => {
  const slint = await readFile(new URL("../ui/bar.slint", import.meta.url), "utf8");
  for (const text of [
    '@tr("{} ({})", title, key)',
    '@tr("{}\\nRight-click: {}", with-key(@tr("Back {} seconds", step-text), key-back), with-key(@tr("Play previous"), key-previous))',
    '@tr("{}\\nRight-click: {}", with-key(@tr("Forward {} seconds", step-text), key-forward), with-key(@tr("Play next"), key-next))',
    'with-key(playing ? @tr("Pause") : @tr("Play"), key-play-pause)',
    'with-key(muted ? @tr("Unmute") : @tr("Mute"), key-mute)',
    '@tr("Volume: {}%", volume-percent)',
  ])
    assert.ok(slint.includes(text), text);
});
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
  // Properties are written one at a time: `key-mute` comes after `key-open`.
  await until(() => bar.props["key-open"] === "Ctrl+O" && bar.props["key-mute"] === "M");
  // The test host labels a binding with its input name (the player shows ← → PgUp PgDn).
  let tip = tips(host, bar.props);
  assert.equal(tip.playPause, "暂停 (Space)");
  assert.equal(tip.stop, "停止 (F4)");
  assert.equal(tip.back, "后退 5 秒 (ArrowLeft)\n右键：播放上一个 (PageUp)");
  assert.equal(tip.forward, "前进 5 秒 (ArrowRight)\n右键：播放下一个 (PageDown)");
  assert.equal(tip.open, "打开文件 (Ctrl+O)");
  assert.equal(tip.time, "点击切换剩余时间");
  assert.equal(tip.mute, "静音 (M)");
  assert.equal(tip.volume, "音量: 100%");
  // The sidebar toggle is the player's (BarButtons, Toggle Sidebar): the bar sends no key for it.
  assert.equal("key-sidebar" in bar.props, false);

  host.setSource({ paused: true, output: { volume: 35, muted: true } });
  await host.settle();
  tip = tips(host, bar.props);
  assert.equal(tip.playPause, "播放 (Space)");
  assert.equal(tip.mute, "取消静音 (M)");
  assert.equal(tip.volume, "音量: 35%");

  // A rebound key shows at once (the table is read again on bindings.changed).
  host.bindings.overlay.set("host/play-pause", ["K"]);
  host.bindings.overlay.set("host/previous", []);
  host.emit("bindings.changed", null);
  // Each overlay change may announce the table on its own: wait for both.
  await until(() => bar.props["key-play-pause"] === "K" && bar.props["key-previous"] === "");
  assert.equal(tips(host, bar.props).playPause, "播放 (K)");
  assert.equal(tips(host, bar.props).back, "后退 5 秒 (ArrowLeft)\n右键：播放上一个");
  await stop();
});

test("G-07 in English the tooltips are the source texts", async () => {
  const { host, stop } = await start(withMedia(), undefined, "en");
  const bar = host.mounted();
  host.emit("bindings.changed", null);
  await until(() => bar.props["key-open"] === "Ctrl+O");
  const tip = tips(host, bar.props);
  assert.equal(tip.playPause, "Pause (Space)");
  assert.equal(tip.back, "Back 5 seconds (ArrowLeft)\nRight-click: Play previous (PageUp)");
  assert.equal(tip.volume, "Volume: 100%");
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
    assert.equal(bar.props["remaining-time"], true);
    assert.equal(tips(first.host, bar.props).time, "点击切换已播放时间");
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
    assert.equal(second.host.mounted().props["step-text"], "0.5");
    assert.match(tips(second.host, second.host.mounted().props).forward, /^前进 0\.5 秒 /);
    await second.stop();
  } finally {
    rmSync(data, { recursive: true, force: true });
  }
});

