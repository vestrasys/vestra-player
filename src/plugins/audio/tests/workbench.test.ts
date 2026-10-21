import assert from "node:assert/strict";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { presets } from "../model.ts";

const directory = new URL("..", import.meta.url);
const track = (trackId: number, selected = true) => ({
  trackId,
  streamIndex: trackId,
  selected,
  external: false,
  codec: "aac",
  title: null,
  language: null,
});
const replaces = (host: TestHost) => host.sent("processing.replace");
const latest = (host: TestHost) => replaces(host).at(-1)!;
const own = (host: TestHost) => host.processing.resources.get("audio:workbench");
/** Let handlers and bindings run while a request is held (`host.settle()` waits for it). */
const flush = async () => {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 1));
};

/** A ready source with one selected track, the plugin started and its window open. */
async function open(host = new TestHost()) {
  host.openElsewhere("/movie.mp4");
  host.setSource({ audioTracks: [track(1)] });
  const started = await startPlugin(activate, { directory, host });
  await host.command("audio.workbench").run();
  return { ...started, window: host.window("audio-workbench") };
}

/** Holds every replace until `release()`; then runs `then` for each held request. */
function hold(host: TestHost, then: (params: Record<string, unknown>) => unknown = () => undefined) {
  const held: (() => void)[] = [];
  host.handle("processing.replace", (params) =>
    new Promise((resolve, reject) =>
      held.push(() => {
        try {
          resolve(then(params));
        } catch (error) {
          reject(error);
        }
      }),
    ),
  );
  return {
    count: () => held.length,
    async release() {
      host.reset("processing.replace");
      for (const next of held.splice(0)) next();
      await host.settle();
    },
  };
}

test("menu under 声音 follows the allocation, and editing commands wait for a selected track", async () => {
  const { host, stop } = await startPlugin(activate, { directory });
  await host.settle();
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d }) => [d.path.join(" ▶ "), d.order, d.group, d.bindings.join(","), d.radio, d.repeat]),
    [
      ["打开 ▶ 载入外部音频…", 114, "open", "", false, false],
      ["声音 ▶ 音频工作台…", 510, "audio-tools", "", false, false],
      ["声音 ▶ 音轨 ▶ 按序切换", 512, "audio-track-cycle", "Alt+A", false, false],
      ...["立体声", "左声道", "右声道", "单声道混合", "左右互换"].map((label) => [`声音 ▶ 音轨 ▶ ${label}`, 512, "audio-channels", "", true, false]),
      ["声音 ▶ 均衡器", 513, "audio-eq", "Shift+E", false, false],
      ...presets.map((p) => [`声音 ▶ 预设 ▶ ${p.label}`, 514, "audio-eq", "", true, false]),
      ["声音 ▶ 声音同步 ▶ 超前 50 毫秒", 515, "audio-eq", ">", false, true],
      ["声音 ▶ 声音同步 ▶ 滞后 50 毫秒", 515, "audio-eq", "<", false, true],
      ["声音 ▶ 声音同步 ▶ 复位", 515, "audio-eq", "?", false, true],
      ["声音 ▶ 声音同步 ▶ 不保持", 515, "audio-sync-keep", "", true, false],
      ["声音 ▶ 声音同步 ▶ 本次运行内保持", 515, "audio-sync-keep", "", true, false],
      ["声音 ▶ 声音同步 ▶ 始终保持", 515, "audio-sync-keep", "", true, false],
      ["声音 ▶ 声音处理 ▶ 规格化", 516, "audio-eq", "Shift+N", false, false],
      ["声音 ▶ 声音处理 ▶ 晶化", 516, "audio-eq", "Shift+C", false, false],
      ["声音 ▶ 复位", 517, "audio-eq", "", false, false],
      ["声音 ▶ 音轨 ▶ 没有音轨", 511, "audio-tools", "", false, false],
      ["声音 ▶ 输出设备 ▶ 没有可用的输出设备", 512, "audio-tools", "", false, false],
    ],
  );
  assert.equal(host.command("audio.device.none").state.enabled, false);
  assert.equal(host.command("audio.track.none").state.enabled, false);
  assert.equal(host.command("audio.load").state.reason, "没有正在播放的文件");
  assert.equal(host.command("audio.track.cycle").state.reason, "没有音轨");
  assert.deepEqual(host.conflicts, []);
  assert.deepEqual(host.command("audio.workbench").state, { enabled: true, checked: null, busy: false, reason: null });
  assert.deepEqual(host.command("audio.reset").state, {
    enabled: false,
    checked: null,
    busy: false,
    reason: "没有选中的音轨",
  });
  assert.equal(host.command("audio.equalizer").state.enabled, false);
  for (const key of ["Shift+E", ">", "<", "?", "Shift+N", "Shift+C"]) await assert.rejects(host.press(key), key);
  assert.equal(host.command("audio.sync.earlier").state.reason, "没有选中的音轨");
  assert.equal(host.command("audio.sync.keep.none").state.checked, true);
  assert.equal(host.command("audio.sync.keep.always").state.enabled, true, "the keep mode needs no track");
  assert.equal(host.command("audio.preset.0").state.checked, true);
  await host.command("audio.workbench").run();
  const window = host.window("audio-workbench");
  assert.equal(window.props["status-text"], "没有选中的音轨");
  assert.equal(window.props["source-ready"], false);
  window.emit("delay-changed", 500); // disabled controls: ignored
  host.openElsewhere("/movie.mp4", "loading");
  await host.settle();
  assert.equal(window.props["status-text"], "正在载入音频…");
  host.setSource({ phase: "ready", audioTracks: [track(1, false)] });
  await host.settle();
  assert.equal(window.props["status-text"], "没有选中的音轨");
  host.setSource({ audioTracks: [track(1)] });
  await host.settle();
  assert.equal(window.props["status-text"], "就绪");
  assert.equal(window.props["source-ready"], true);
  assert.equal(host.command("audio.reset").state.enabled, true);
  assert.equal(replaces(host).length, 0, "a never-applied neutral chain is not sent");
  await stop();
});

test("edits round like the host: EQ ±6 dB in 0.1 dB, delay to 10 ms, 晶化 0–6 dB", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 0, 9.87);
  window.emit("gain-changed", 1, -2.345);
  window.emit("gain-changed", 6, 3); // no such band
  window.emit("gain-changed", 2, Number.NaN); // rejected, nothing changes
  window.emit("delay-changed", 104.9);
  window.emit("clarity-changed", 5); // 晶化 is off: the slider is greyed out, nothing changes
  window.emit("clarity-toggled", true);
  window.emit("clarity-changed", 7);
  window.emit("normalize-changed", true);
  await host.settle();
  assert.deepEqual(window.props["gain-values"], [6, -2.3, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 100);
  assert.equal(window.props["clarity-db"], 6);
  assert.equal(window.props["normalize-enabled"], true);
  assert.deepEqual(own(host)?.nodes, [
    { kind: "equalizer", frequency: 60, q: 1, gainDb: 6 },
    { kind: "equalizer", frequency: 170, q: 1, gainDb: -2.3 },
    { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 6 },
    { kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 },
  ]);
  assert.equal(own(host)?.offsetMs, 100);
  window.emit("delay-changed", 105);
  await host.settle();
  assert.equal(own(host)?.offsetMs, 110);
  window.emit("delay-changed", -15000);
  await host.settle();
  assert.equal(own(host)?.offsetMs, -10000, "G09: ±10 000 ms");
  assert.equal(window.props["status-text"], "已应用");
  assert.equal(host.sent("playback.set").length + host.sent("source.submit").length, 0, "no volume, mute or transport");
  await stop();
});

test("a burst of edits keeps one change in flight and ends on the last value", async () => {
  const { host, window, stop } = await open();
  const gate = hold(host);
  for (const delay of [100, 200, 300]) window.emit("delay-changed", delay);
  await flush();
  assert.equal(gate.count(), 1);
  assert.equal(window.props["is-busy"], true);
  assert.equal(window.props["status-text"], "正在应用…");
  assert.equal(host.command("audio.reset").state.busy, true);
  await gate.release();
  assert.equal(own(host)?.offsetMs, 300);
  assert.ok(replaces(host).length <= 2);
  assert.equal(window.props["is-busy"], false);
  assert.equal(host.command("audio.reset").state.busy, false);
  await stop();
});

test("presets change the EQ only and the window follows them after a drag; reset clears the chain", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 3, -4); // the user dragged a band slider
  window.emit("delay-changed", 250);
  window.emit("preset-selected", 1);
  await host.settle();
  assert.deepEqual(window.props["gain-values"], [...presets[1].gains]);
  assert.equal(window.props["delay-ms"], 250);
  assert.equal(host.command("audio.preset.1").state.checked, true);
  assert.equal(host.command("audio.preset.0").state.checked, false);
  await host.command("audio.preset.2").run();
  assert.deepEqual(window.props["gain-values"], [...presets[2].gains]);
  assert.equal(host.command("audio.preset.2").state.checked, true);
  window.emit("reset-requested");
  await host.settle();
  assert.deepEqual(window.props["gain-values"], [0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 0);
  assert.deepEqual(own(host)?.nodes, []);
  assert.equal(own(host)?.offsetMs, 0);
  assert.equal(host.command("audio.preset.0").state.checked, true);
  window.emit("delay-changed", 40);
  await host.settle();
  await host.command("audio.reset").run();
  assert.equal(own(host)?.offsetMs, 0);
  assert.equal(window.props["delay-ms"], 0);
  await stop();
});

test("under 不保持 another load of a source resets only the delay; an old success does not report", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 0, 3);
  await host.settle();
  const gate = hold(host);
  window.emit("delay-changed", 500);
  await flush();
  host.openElsewhere("/movie.mp4");
  host.setSource({ audioTracks: [track(1)] });
  await flush();
  assert.equal(window.props["delay-ms"], 0);
  await gate.release();
  assert.equal(latest(host).expectedGeneration, 2);
  assert.equal(latest(host).offsetMs, 0);
  assert.deepEqual(latest(host).nodes, [{ kind: "equalizer", frequency: 60, q: 1, gainDb: 3 }], "the equalizer is global");
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0]);
  assert.equal(window.props["status-text"], "已应用");
  await stop();
});

test("a track switch in the same source keeps the delay and the effects", async () => {
  const { host, window, stop } = await open();
  window.emit("delay-changed", 500);
  window.emit("gain-changed", 0, 3);
  await host.settle();
  host.setSource({ audioTracks: [track(1, false), track(2)] });
  await host.settle();
  assert.equal(own(host)?.offsetMs, 500);
  assert.deepEqual(own(host)?.nodes, [{ kind: "equalizer", frequency: 60, q: 1, gainDb: 3 }]);
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 500);
  assert.equal(window.props["status-text"], "已应用");
  await stop();
});

test(
  "a track switch ignores the old track's in-flight failure and applies the latest value",
  async () => {
    const { host, window, stop } = await open();
    window.emit("delay-changed", 300); // the old track already has an applied chain
    await host.settle();
    const gate = hold(host, () => {
      throw new HostError("backend", "old-track failure");
    });
    window.emit("delay-changed", 500);
    await flush();
    window.emit("delay-changed", 900);
    host.setSource({ audioTracks: [track(1, false), track(2)] });
    await flush();
    await gate.release();
    assert.equal(window.props["has-error"], false);
    assert.equal(own(host)?.offsetMs, 900);
    assert.equal(window.props["status-text"], "已应用");
    await stop();
  },
);

test("a host validation refusal is shown, never sticks, and retry or the next edit applies", async () => {
  const { host, window, stop } = await open();
  host.handle("processing.replace", () => {
    throw new HostError("invalid", "offset out of range");
  });
  window.emit("delay-changed", 105);
  await host.settle();
  assert.equal(window.props["has-error"], true);
  assert.equal(window.props["status-text"], "音频处理失败");
  assert.equal(window.props["error-detail"], "offset out of range");
  assert.equal(host.command("audio.reset").state.busy, false);
  host.setSource({ position: 3 }); // ordinary source updates keep the error, nothing "pending"
  await host.settle();
  assert.equal(window.props["status-text"], "音频处理失败");
  host.reset("processing.replace");
  window.emit("retry-requested");
  await host.settle();
  assert.equal(window.props["has-error"], false);
  assert.equal(window.props["error-detail"], "");
  assert.equal(own(host)?.offsetMs, 110);
  host.handle("processing.replace", () => {
    throw new HostError("invalid", "refused again");
  });
  window.emit("delay-changed", 300);
  await host.settle();
  assert.equal(window.props["has-error"], true);
  host.reset("processing.replace");
  window.emit("delay-changed", 200);
  await host.settle();
  assert.equal(window.props["has-error"], false);
  assert.equal(own(host)?.offsetMs, 200);
  assert.equal(window.props["status-text"], "已应用");
  await stop();
});

test("a lost reply is resolved by the SDK: applied or resent, never left pending", async () => {
  const { host, window, stop } = await open();
  // Never admitted: the reply is lost before the host saw it.
  host.handle("processing.replace", () => {
    host.reset("processing.replace");
    throw new Error("transport timeout");
  });
  window.emit("delay-changed", 100);
  await host.settle(150);
  assert.equal(own(host)?.offsetMs, 100);
  assert.equal(window.props["status-text"], "已应用");
  // Admitted and applied, but the reply is lost.
  host.handle("processing.replace", async (params) => {
    host.reset("processing.replace");
    await host.request("processing.replace", params);
    throw new Error("reply lost");
  });
  window.emit("delay-changed", 900);
  await host.settle();
  assert.equal(own(host)?.offsetMs, 900);
  assert.equal(window.props["has-error"], false);
  assert.equal(window.props["status-text"], "已应用");
  window.emit("delay-changed", 200);
  await host.settle();
  assert.equal(own(host)?.offsetMs, 200);
  await stop();
});

test("an out-of-order older source event cannot restore an older source", async () => {
  const { host, window, stop } = await open();
  const older = structuredClone(host.source);
  host.openElsewhere("/other.mp4");
  host.setSource({ audioTracks: [track(1)] });
  await host.settle();
  host.emit("source.state", older);
  await host.settle();
  window.emit("delay-changed", 100);
  await host.settle();
  assert.equal(latest(host).expectedGeneration, 2);
  assert.equal(own(host)?.offsetMs, 100);
  await stop();
});

test("Shift+E bypasses the bands but keeps their gains; presets and band edits turn it back on", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 0, 3);
  window.emit("clarity-toggled", true);
  window.emit("clarity-changed", 2);
  await host.settle();
  assert.equal(window.props["preset-index"], -1);
  await host.press("Shift+E");
  await host.settle();
  assert.equal(host.osd.at(-1), "均衡器: 关");
  assert.equal(host.command("audio.equalizer").state.checked, false);
  assert.equal(window.props["equalizer-enabled"], false);
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0]);
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 2 }]);
  await host.press("Shift+E");
  await host.settle();
  assert.equal(host.osd.at(-1), "均衡器: 开");
  assert.equal((own(host)?.nodes as { kind: string }[]).length, 2);
  window.emit("equalizer-changed", false);
  await host.settle();
  assert.equal(host.command("audio.equalizer").state.checked, false);
  window.emit("gain-changed", 1, -2);
  await host.settle();
  assert.equal(window.props["equalizer-enabled"], true);
  window.emit("equalizer-changed", false);
  await host.command("audio.preset.1").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "均衡器预设: 人声");
  assert.equal(window.props["equalizer-enabled"], true);
  assert.equal(window.props["preset-index"], 1);
  assert.equal(host.command("audio.preset.1").state.checked, true);
  await host.command("audio.reset").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "音频处理已复位");
  assert.deepEqual(own(host)?.nodes, []);
  assert.equal(window.props["preset-index"], 0);
  assert.equal(host.osd.length, 4, "window edits give no on-screen text");
  await stop();
});

test("headless: a missing on-screen display does not break the menu commands", async () => {
  const host = new TestHost();
  host.handle("osd.show", () => {
    throw new HostError("unavailable", "headless");
  });
  const { window, stop } = await open(host);
  await host.press("Shift+E");
  await host.command("audio.preset.2").run();
  await host.settle();
  assert.deepEqual(window.props["gain-values"], [...presets[2].gains]);
  assert.equal(window.props["has-error"], false);
  await stop();
});

test("G09: > < ? step the audio sync by 50 ms with OSD, stop at ±10 000 ms and move the window slider", async () => {
  const { host, window, stop } = await open();
  await host.press(">");
  assert.equal(host.osd.at(-1), "音频同步: -0.05秒");
  await host.press(">");
  await host.press(">");
  assert.equal(host.osd.at(-1), "音频同步: -0.15秒");
  await host.settle();
  assert.equal(window.props["delay-ms"], -150);
  assert.equal(own(host)?.offsetMs, -150);
  for (let i = 0; i < 9; i++) await host.press("<");
  assert.equal(host.osd.at(-1), "音频同步: +0.3秒");
  await host.press("?");
  assert.equal(host.osd.at(-1), "音频同步: 0秒");
  await host.settle();
  assert.equal(own(host)?.offsetMs, 0);
  window.emit("delay-changed", 9980);
  await host.press("<");
  await host.press("<");
  assert.equal(host.osd.at(-1), "音频同步: +10秒", "the edge keeps the value and still reports it");
  await host.settle();
  assert.equal(own(host)?.offsetMs, 10000);
  window.emit("delay-reset");
  await host.settle();
  assert.equal(own(host)?.offsetMs, 0);
  assert.equal(host.sent("subtitles.style").length, 0, "audio sync never touches the subtitles");
  await stop();
});

test("G09: sync keys are not swallowed while a change is applying", async () => {
  const { host, stop } = await open();
  const gate = hold(host);
  // A press resolves once the host is idle, which a held change never is: do not await it.
  const presses = [host.press(">")];
  await flush();
  assert.equal(gate.count(), 1);
  presses.push(host.press(">"), host.press(">"));
  await flush();
  assert.equal(host.osd.at(-1), "音频同步: -0.15秒");
  await gate.release();
  await Promise.all(presses);
  assert.equal(own(host)?.offsetMs, -150);
  await stop();
});

test("G09: 本次运行内保持 keeps the delay across sources; 始终保持 also keeps it across restarts", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    const first = await open(new TestHost());
    await first.host.command("audio.sync.keep.session").run();
    assert.equal(first.host.command("audio.sync.keep.session").state.checked, true);
    first.window.emit("delay-changed", 300);
    await first.host.settle();
    first.host.openElsewhere("/next.mp4");
    first.host.setSource({ audioTracks: [track(1)] });
    await first.host.settle();
    assert.equal(first.window.props["delay-ms"], 300);
    assert.equal(own(first.host)?.offsetMs, 300);
    await first.stop();

    // 本次运行内保持: a restart starts at 0.
    let started = await startPlugin(activate, { directory, dataDirectory: data });
    await started.host.command("audio.sync.keep.session").run();
    started.host.openElsewhere("/movie.mp4");
    started.host.setSource({ audioTracks: [track(1)] });
    await started.host.settle();
    await started.host.press("<");
    await started.stop();
    started = await startPlugin(activate, { directory, dataDirectory: data });
    assert.equal(started.host.command("audio.sync.keep.session").state.checked, true, "the mode itself is saved");
    await started.host.command("audio.workbench").run();
    assert.equal(started.host.window("audio-workbench").props["delay-ms"], 0);

    // 始终保持: the delay is written and comes back.
    await started.host.command("audio.sync.keep.always").run();
    started.host.openElsewhere("/movie.mp4");
    started.host.setSource({ audioTracks: [track(1)] });
    await started.host.settle();
    await started.host.press("<");
    await started.host.press("<");
    await started.stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.version, 1);
    assert.equal(saved.syncKeep, "always");
    assert.equal(saved.syncMs, 100);
    const host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.equal(own(host)?.offsetMs, 100);
    await host.command("audio.workbench").run();
    const window = host.window("audio-workbench");
    assert.equal(window.props["delay-ms"], 100);
    assert.equal(window.props["sync-keep-index"], 2);
    window.emit("sync-keep-selected", 0);
    await started.stop();
    assert.equal(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).syncMs, undefined);
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("G11: Shift+C switches 晶化 at the remembered strength; menu, window and OSD agree", async () => {
  const { host, window, stop } = await open();
  assert.equal(window.props["clarity-enabled"], false);
  assert.equal(window.props["clarity-db"], 3, "default strength 3 dB");
  await host.press("Shift+C");
  await host.settle();
  assert.equal(host.osd.at(-1), "晶化: 开");
  assert.equal(host.command("audio.clarity").state.checked, true);
  assert.equal(window.props["clarity-enabled"], true);
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 }]);
  window.emit("clarity-changed", 4.56);
  await host.settle();
  assert.equal(window.props["clarity-db"], 4.6);
  await host.press("Shift+C");
  await host.settle();
  assert.equal(host.osd.at(-1), "晶化: 关");
  assert.deepEqual(own(host)?.nodes, []);
  assert.equal(window.props["clarity-db"], 4.6, "off keeps the strength");
  window.emit("clarity-toggled", true);
  await host.settle();
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 4.6 }]);
  window.emit("clarity-changed", 0);
  await host.settle();
  assert.equal(host.command("audio.clarity").state.checked, true, "on at 0 dB stays checked");
  assert.deepEqual(own(host)?.nodes, [], "but makes no node");
  window.emit("clarity-reset");
  await host.settle();
  assert.equal(window.props["clarity-enabled"], false);
  assert.equal(window.props["clarity-db"], 3);
  await host.press("Shift+N");
  await host.settle();
  assert.equal(host.osd.at(-1), "规格化: 开");
  assert.equal(window.props["normalize-enabled"], true);
  await stop();
});

test("global settings and the last page survive a restart", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    let host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.command("audio.preset.1").run();
    await host.press("Shift+C");
    await host.command("audio.workbench").run();
    host.window("audio-workbench").emit("tab-changed", 1);
    host.window("audio-workbench").emit("delay-changed", 400); // 不保持: not saved
    await started.stop();
    host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.command("audio.workbench").run();
    const window = host.window("audio-workbench");
    assert.equal(window.props["tab-index"], 1);
    assert.equal(window.props["preset-index"], 1);
    assert.equal(window.props["clarity-enabled"], true);
    assert.equal(window.props["delay-ms"], 0);
    assert.equal(window.props["status-text"], "已应用", "saved settings are applied at start");
    assert.equal((own(host)?.nodes as unknown[]).length, 7);
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("a broken settings file is kept aside and reads as the defaults", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    await writeFile(join(data, "settings.json"), "{ not json");
    const { host, stop } = await startPlugin(activate, { directory, dataDirectory: data });
    assert.equal(host.command("audio.preset.0").state.checked, true);
    assert.equal(host.command("audio.sync.keep.none").state.checked, true);
    assert.ok((await readdir(data)).some((name) => name.startsWith("settings.json.bad-")));
    await stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------- round 2: tracks, channels, external audio

const titled = (trackId: number, title: string, language: string | null, selected: boolean) => ({
  ...track(trackId, selected),
  title,
  language,
});

test("G03: the 音轨 menu lists the tracks, follows the list and switches with OSD; stale is silent", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [titled(1, "日语", "jpn", true), titled(2, "English", "eng", false)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  const items = () =>
    [...host.commands.values()]
      .filter(({ declaration: d }) => d.path[1] === "音轨" && d.order === 511)
      .map(({ declaration: d }) => d.path[2]);
  assert.deepEqual(items(), ["1. 日语 [jpn] (aac)", "2. English [eng] (aac)"]);
  assert.equal(host.command("audio.track.1").state.checked, true);
  assert.equal(host.command("audio.track.1").declaration.radio, true);
  await host.command("audio.track.2").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "音轨: 2. English [eng] (aac)");
  assert.equal(host.command("audio.track.2").state.checked, true);
  assert.equal(host.command("audio.track.1").state.checked, false);
  const sent = host.sent("audio.selectTrack").length;
  await host.command("audio.track.2").run();
  assert.equal(host.sent("audio.selectTrack").length, sent, "the selected track sends nothing");
  host.handle("audio.selectTrack", () => {
    throw new HostError("stale", "source changed");
  });
  const osd = host.osd.length;
  await host.command("audio.track.1").run();
  assert.equal(host.osd.length, osd, "stale says nothing");
  host.reset("audio.selectTrack");
  // A new source rebuilds the list; while it loads the menu says so.
  host.openElsewhere("/next.mkv", "loading");
  host.setSource({ audioTracks: null });
  await host.settle();
  assert.deepEqual(items(), ["正在载入…"]);
  assert.equal(host.command("audio.track.cycle").state.reason, "正在载入");
  host.setSource({ phase: "ready", audioTracks: [] });
  await host.settle();
  assert.deepEqual(items(), ["没有音轨"]);
  await stop();
});

test("G04: Alt+A cycles in list order and wraps; one track says so", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [track(1, true), track(2, false), track(3, false)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  for (let i = 0; i < 3; i++) {
    await host.press("Alt+A");
    await host.settle();
  }
  assert.deepEqual(host.osd, ["音轨: 音轨 2 (aac)", "音轨: 音轨 3 (aac)", "音轨: 音轨 1 (aac)"]);
  assert.deepEqual(host.sent("audio.selectTrack").map((p) => p.id), [2, 3, 1]);
  host.setSource({ audioTracks: [track(1, true)] });
  await host.settle();
  await host.press("Alt+A");
  assert.equal(host.osd.at(-1), "只有一条音轨");
  assert.equal(host.sent("audio.selectTrack").length, 3);
  await stop();
});

test("G05: channel modes route a stereo mix, are global, and wait on a mono track", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, audio: { codec: "aac", sampleRate: 48000, channels: 6, decoder: "aac", format: "floatp", channelLayout: "5.1", bitrate: null } };
  const { window, stop } = await open(host);
  await host.settle();
  await host.command("audio.channels.left").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "声道: 左声道（已混为立体声）");
  assert.deepEqual(own(host)?.nodes, [{ kind: "channels", mode: "left" }]);
  assert.equal(window.props["channel-index"], 1);
  assert.equal(host.command("audio.channels.left").state.checked, true);
  window.emit("gain-changed", 0, 2);
  window.emit("channel-selected", 4);
  await host.settle();
  assert.deepEqual((own(host)?.nodes as { kind: string }[]).map((n) => n.kind), ["channels", "equalizer"]);
  assert.deepEqual((own(host)?.nodes as unknown[])[0], { kind: "channels", mode: "swap" });
  // Another source keeps the mode; a mono track keeps it but sends no node.
  host.stats = { ...host.stats, audio: { ...host.stats.audio!, channels: 1 } };
  host.openElsewhere("/mono.wav");
  host.setSource({ audioTracks: [track(1)] });
  await host.settle(20);
  assert.deepEqual((own(host)?.nodes as { kind: string }[]).map((n) => n.kind), ["equalizer"]);
  assert.equal(host.command("audio.channels.mono").state.enabled, false);
  assert.equal(host.command("audio.channels.mono").state.reason, "当前音轨是单声道");
  assert.equal(window.props["channels-available"], false);
  assert.equal(window.props["channel-tip"], "当前音轨是单声道");
  assert.equal(window.props["channel-index"], 4, "the setting is kept");
  host.stats = { ...host.stats, audio: { ...host.stats.audio!, channels: 2 } };
  host.setSource({ audioTracks: [track(1, false), track(2)] });
  await host.settle(20);
  assert.deepEqual((own(host)?.nodes as unknown[])[0], { kind: "channels", mode: "swap" });
  await host.command("audio.channels.stereo").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "声道: 立体声");
  assert.deepEqual((own(host)?.nodes as { kind: string }[]).map((n) => n.kind), ["equalizer"]);
  await stop();
});

test("A11: 载入外部音频… adds and selects the file once; failures and a changed source are reported", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [track(1)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("audio.workbench").run();
  const window = host.window("audio-workbench");
  assert.equal(host.command("audio.load").state.enabled, true);
  assert.equal(window.props["can-load"], true);
  host.dialogAnswers.push(null); // cancelled: nothing happens
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.sent("audio.addTrack").length, 0);
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  window.emit("load-requested");
  await host.settle();
  assert.equal(host.osd.at(-1), "已载入音频: movie.jpn.mka");
  assert.deepEqual(host.sent("audio.addTrack").at(-1)!.path, "/music/movie.jpn.mka");
  assert.equal(host.sent("audio.addTrack").at(-1)!.select, true, "selected");
  assert.deepEqual(window.props["track-names"], ["音轨 1 (aac)", "2. movie.jpn.mka（外部）"]);
  assert.equal(window.props["track-index"], 1);
  assert.equal(host.command("audio.track.2").state.checked, true);
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "该音频已载入");
  assert.equal(host.sent("audio.addTrack").length, 1);
  host.handle("audio.addTrack", () => {
    throw new HostError("backend", "unrecognized file format");
  });
  host.dialogAnswers.push(["/music/notes.txt"]);
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "无法载入音频: unrecognized file format");
  host.reset("audio.addTrack");
  // The source changes while the dialog is open.
  host.handle("dialogs.open", () => {
    host.openElsewhere("/next.mkv");
    host.setSource({ audioTracks: [track(1)] });
    return { cancelled: false, paths: ["/music/next.mp3"] };
  });
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "来源已改变，未载入音频");
  assert.deepEqual(host.source.audioTracks?.map((t) => t.trackId), [1]);
  host.reset("dialogs.open");
  host.openElsewhere("/later.mkv", "loading");
  await host.settle();
  assert.equal(host.command("audio.load").state.reason, "正在载入");
  assert.equal(window.props["can-load"], false);
  await stop();
});

test("A11: 自动载入同名音频文件 adds same-named audio next to a local file without selecting it", async () => {
  const folder = await mkdtemp(join(tmpdir(), "vestra-audio-media-"));
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    await mkdir(join(folder, "sub"));
    for (const name of ["film.mkv", "film.ac3", "film.jpn.mka", "film.srt", "other.mp3", "sub/film.flac"]) await writeFile(join(folder, name), "");
    const host = new TestHost();
    host.openElsewhere(join(folder, "film.mkv"));
    host.setSource({ audioTracks: [track(1)] });
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.equal(host.sent("audio.addTrack").length, 0, "off by default");
    await host.command("audio.workbench").run();
    host.window("audio-workbench").emit("auto-load-changed", true);
    await host.settle(20);
    assert.deepEqual(host.sent("audio.addTrack").map((p) => [p.path, p.select]), [
      [join(folder, "film.ac3"), false],
      [join(folder, "film.jpn.mka"), false],
    ]);
    assert.equal(host.source.audioTracks?.find((t) => t.selected)?.trackId, 1, "the selection is kept");
    assert.deepEqual(host.osd, [], "silent");
    await started.stop();
    // Remembered: the next local file is scanned when it is ready; a URL is not.
    const next = new TestHost();
    next.openElsewhere("https://example.com/film.mkv");
    next.setSource({ audioTracks: [track(1)] });
    started = await startPlugin(activate, { directory, host: next, dataDirectory: data });
    await next.settle(20);
    assert.equal(next.sent("audio.addTrack").length, 0);
    next.openElsewhere(join(folder, "film.mkv"));
    next.setSource({ audioTracks: [track(1)] });
    await next.settle(20);
    assert.equal(next.sent("audio.addTrack").length, 2);
    await started.stop();
  } finally {
    await rm(folder, { recursive: true, force: true });
    await rm(data, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------- round 3a: G16 output device

const SPEAKERS = { id: "coreaudio/BuiltInSpeakerDevice", name: "MacBook Air 扬声器" };
const PHONES = { id: "coreaudio/USB-1", name: "USB 耳机" };
const deviceItems = (host: TestHost) =>
  [...host.commands.entries()]
    .filter(([, { declaration: d }]) => d.path[1] === "输出设备")
    .map(([id, { declaration: d, state }]) => [d.path[2], d.order, d.group, d.radio, state.enabled, state.checked, state.reason, id] as const);
const deviceCommand = (host: TestHost, title: string) => host.command(deviceItems(host).find((item) => item[0] === title)![7]);
const savedJson = async (path: string) => JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
/** The player noticed a plug or unplug (it checks every second): what `audioOutput` becomes. */
function plug(host: TestHost, devices: { id: string; name: string }[]) {
  host.audioDevices = devices;
  const requested = host.source.audioOutput?.requested ?? null;
  host.setSource({ audioOutput: { requested, active: requested !== null && devices.some((d) => d.id === requested) ? requested : null } });
}

test("G16: 输出设备 lists 系统默认 and the devices; choosing one switches, says so and is saved", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    const host = new TestHost();
    host.audioDevices = [SPEAKERS, PHONES];
    const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.deepEqual(deviceItems(host).map((item) => item.slice(0, 7)), [
      ["系统默认", 512, "audio-tools", true, true, true, null],
      ["MacBook Air 扬声器", 512, "audio-tools", true, true, false, null],
      ["USB 耳机", 512, "audio-tools", true, true, false, null],
    ]);
    assert.equal(host.sent("player.setAudioDevice").length, 0, "系统默认 at start sends nothing");
    await deviceCommand(host, "USB 耳机").run();
    await host.settle();
    assert.deepEqual(host.sent("player.setAudioDevice").at(-1), { id: PHONES.id });
    assert.deepEqual(host.source.audioOutput, { requested: PHONES.id, active: PHONES.id });
    assert.equal(host.osd.at(-1), "音频输出: USB 耳机");
    assert.equal(deviceCommand(host, "USB 耳机").state.checked, true);
    assert.equal(deviceCommand(host, "系统默认").state.checked, false);
    // No source is needed, and the window shows the same choice.
    await host.command("audio.workbench").run();
    const window = host.window("audio-workbench");
    assert.deepEqual(window.props["device-names"], ["系统默认", "MacBook Air 扬声器", "USB 耳机"]);
    assert.equal(window.props["device-index"], 2);
    assert.equal(window.props["devices-available"], true);
    assert.equal(window.props["devices-tip"], "USB 耳机");
    window.emit("device-selected", 0);
    await host.settle();
    assert.deepEqual(host.sent("player.setAudioDevice").at(-1), { id: null });
    assert.equal(host.osd.at(-1), "音频输出: 系统默认");
    assert.equal(window.props["device-index"], 0);
    window.emit("device-selected", 1);
    await host.settle(350);
    assert.equal((await savedJson(join(data, "settings.json"))).device, SPEAKERS.id);
    assert.equal((await savedJson(join(data, "settings.json"))).deviceName, SPEAKERS.name);
    // A refused switch keeps the previous device.
    host.handle("player.setAudioDevice", () => {
      throw new HostError("busy", "设备被占用");
    });
    await deviceCommand(host, "USB 耳机").run();
    await host.settle();
    assert.equal(host.osd.at(-1), "无法切换到 USB 耳机: 设备被占用");
    assert.equal(deviceCommand(host, "MacBook Air 扬声器").state.checked, true);
    await stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("G16: unplugging the chosen device falls back with an OSD; plugging it back returns with one", async () => {
  const host = new TestHost();
  host.audioDevices = [SPEAKERS, PHONES];
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  await deviceCommand(host, "USB 耳机").run();
  await host.settle();
  plug(host, [SPEAKERS]);
  await host.settle();
  assert.equal(host.osd.at(-1), "输出设备已断开，改用系统默认");
  // The check mark stays on the choice, now disabled and listed after the connected ones.
  assert.deepEqual(deviceItems(host).map((item) => item.slice(0, 7)), [
    ["系统默认", 512, "audio-tools", true, true, false, null],
    ["MacBook Air 扬声器", 512, "audio-tools", true, true, false, null],
    ["USB 耳机", 512, "audio-tools", true, false, true, "设备未连接"],
  ]);
  await host.command("audio.workbench").run();
  const window = host.window("audio-workbench");
  assert.deepEqual(window.props["device-names"], ["系统默认", "MacBook Air 扬声器", "USB 耳机（未连接）"]);
  assert.equal(window.props["device-index"], 2);
  assert.equal(window.props["devices-tip"], "USB 耳机：设备未连接，正在使用系统默认");
  const sent = host.sent("player.setAudioDevice").length;
  window.emit("device-selected", 2); // the disconnected entry cannot be picked
  await host.settle();
  assert.equal(host.sent("player.setAudioDevice").length, sent);
  plug(host, [SPEAKERS, PHONES]);
  await host.settle();
  assert.equal(host.osd.at(-1), "音频输出: USB 耳机");
  assert.equal(deviceCommand(host, "USB 耳机").state.enabled, true);
  assert.deepEqual(window.props["device-names"], ["系统默认", "MacBook Air 扬声器", "USB 耳机"]);
  await stop();
});

test("G16: the saved device is restored at start without an OSD, also while it is not connected", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, device: PHONES.id, deviceName: PHONES.name }));
    let host = new TestHost();
    host.audioDevices = [SPEAKERS, PHONES];
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.deepEqual(host.sent("player.setAudioDevice"), [{ id: PHONES.id }]);
    assert.deepEqual(host.source.audioOutput, { requested: PHONES.id, active: PHONES.id });
    assert.equal(deviceCommand(host, "USB 耳机").state.checked, true);
    assert.deepEqual(host.osd, []);
    await started.stop();
    // Not plugged in at start: the player uses the default, the menu says why, no OSD.
    host = new TestHost();
    host.audioDevices = [SPEAKERS];
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.deepEqual(host.source.audioOutput, { requested: PHONES.id, active: null });
    assert.deepEqual(deviceItems(host).at(-1)!.slice(0, 7), ["USB 耳机", 512, "audio-tools", true, false, true, "设备未连接"]);
    assert.deepEqual(host.osd, []);
    plug(host, [SPEAKERS, PHONES]);
    await host.settle();
    assert.equal(host.osd.at(-1), "音频输出: USB 耳机", "plugging it in later is announced");
    assert.equal((await savedJson(join(data, "settings.json"))).device, PHONES.id, "the choice is never dropped");
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("G16: a device plugged in later shows up within 5 s; no list means one disabled item", async () => {
  const host = new TestHost();
  host.handle("player.audioDevices", () => {
    throw new HostError("unavailable", "no audio output");
  });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  assert.deepEqual(deviceItems(host).map((item) => item.slice(0, 7)), [
    ["没有可用的输出设备", 512, "audio-tools", false, false, null, null],
  ]);
  await host.command("audio.workbench").run();
  const window = host.window("audio-workbench");
  assert.deepEqual(window.props["device-names"], ["没有可用的输出设备"]);
  assert.equal(window.props["devices-available"], false);
  host.reset("player.audioDevices");
  host.audioDevices = [SPEAKERS, PHONES];
  await host.settle(5200);
  assert.deepEqual(deviceItems(host).map((item) => item[0]), ["系统默认", "MacBook Air 扬声器", "USB 耳机"]);
  assert.deepEqual(window.props["device-names"], ["系统默认", "MacBook Air 扬声器", "USB 耳机"]);
  assert.deepEqual(host.osd, [], "a new device is not announced");
  await stop();
});
