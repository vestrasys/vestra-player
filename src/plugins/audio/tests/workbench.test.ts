import assert from "node:assert/strict";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
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
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d }) => [d.path.join(" ▶ "), d.order, d.bindings.join(","), d.radio]),
    [
      ["声音 ▶ 音频工作台…", 510, "", false],
      ["声音 ▶ 均衡器", 511, "Shift+E", false],
      ...presets.map((p, i) => [`声音 ▶ 预设 ▶ ${p.label}`, 512 + i, "", true]),
      ["声音 ▶ 复位", 516, "", false],
    ],
  );
  assert.ok([...host.commands.values()].every(({ declaration: d }) => d.group === "domain"));
  assert.deepEqual(host.conflicts, []);
  assert.deepEqual(host.command("audio.workbench").state, { enabled: true, checked: null, busy: false, reason: null });
  assert.deepEqual(host.command("audio.reset").state, {
    enabled: false,
    checked: null,
    busy: false,
    reason: "没有选中的音轨",
  });
  assert.equal(host.command("audio.equalizer").state.enabled, false);
  await assert.rejects(host.press("Shift+E"));
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

test("edits round like the host: EQ ±6 dB in 0.1 dB, delay to 10 ms, clarity 0–6 dB", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 0, 9.87);
  window.emit("gain-changed", 1, -2.345);
  window.emit("gain-changed", 6, 3); // no such band
  window.emit("gain-changed", 2, Number.NaN); // rejected, nothing changes
  window.emit("delay-changed", 104.9);
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
  window.emit("delay-changed", -5000);
  await host.settle();
  assert.equal(own(host)?.offsetMs, -2000);
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

test("another load of a source resets to neutral and an old success does not report", async () => {
  const { host, window, stop } = await open();
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
  assert.deepEqual(latest(host).nodes, []);
  assert.equal(window.props["status-text"], "已应用");
  await stop();
});

test("a track switch in the same source resets to neutral", async () => {
  const { host, window, stop } = await open();
  window.emit("delay-changed", 500);
  window.emit("gain-changed", 0, 3);
  await host.settle();
  host.setSource({ audioTracks: [track(1, false), track(2)] });
  await host.settle();
  assert.equal(latest(host).expectedGeneration, 1);
  assert.equal(own(host)?.offsetMs, 0);
  assert.deepEqual(own(host)?.nodes, []);
  assert.deepEqual(window.props["gain-values"], [0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["status-text"], "已应用");
  await stop();
});

test(
  "a track switch ignores the old track's in-flight failure and applies neutral",
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
    assert.equal(own(host)?.offsetMs, 0);
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
