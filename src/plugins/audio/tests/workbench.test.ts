import assert from "node:assert/strict";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, LOCALES, startPlugin, TestHost } from "@vestra-player/sdk/testing";
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
/** What a drop-down of the window shows: its labels (text values) as the user reads them. */
const listed = (host: TestHost, value: unknown) => (value as Parameters<TestHost["translate"]>[0][]).map((label) => host.translate(label));
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
  return { ...started, window: host.mounted() };
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

test("the right-click menu keeps Load External Audio…, Audio Track ▸ and Audio…; the rest is in the panel, and editing commands wait for a selected track", async () => {
  const { host, stop } = await startPlugin(activate, { directory });
  await host.settle();
  assert.deepEqual(
    [...host.commands.values()].map(({ declaration: d, display }) => [
      d.path.length ? d.path.join(" ▸ ") : display.title,
      d.order,
      d.group,
      d.bindings.join(","),
      d.radio,
      d.repeat,
    ]),
    [
      ["@open ▸ Load External Audio…", 114, "open", "", false, false],
      ["Audio…", 792, "windows", "Alt+2", false, false],
      ["Cycle Tracks", 0, "domain", "Alt+A", false, false],
      ...["Stereo", "Left Channel", "Right Channel", "Mono Mix", "Swap Left and Right"].map((label) => [label, 0, "domain", "", true, false]),
      ["Equalizer", 0, "domain", "Shift+E", false, false],
      ...presets.map((p) => [p.label, 0, "domain", "", true, false]),
      ["50 ms Earlier", 0, "domain", ">", false, true],
      ["50 ms Later", 0, "domain", "<", false, true],
      ["Reset", 0, "domain", "?", false, true],
      ["Don't Keep", 0, "domain", "", true, false],
      ["Keep for This Run", 0, "domain", "", true, false],
      ["Always Keep", 0, "domain", "", true, false],
      ["Normalize", 0, "domain", "Shift+N", false, false],
      ["Crystalizer", 0, "domain", "Shift+C", false, false],
      ["Reset", 0, "domain", "", false, false],
      ["Audio Track ▸ No Audio Tracks", 505, "domain", "", false, false],
    ],
  );
  assert.equal(host.mounted().slot, "adjust");
  assert.equal(host.layout.adjust?.panels?.[0]?.title, "Audio");
  assert.equal(host.layout.adjust?.preferred, false, "the adjust column starts closed");
  assert.equal(host.commands.has("audio.device.none"), false, "no devices: nothing to list, the drop-down says so");
  assert.equal(host.command("audio.track.none").state.enabled, false);
  assert.equal(host.command("audio.load").state.reason, "No file is playing");
  assert.equal(host.command("audio.track.cycle").state.reason, "No audio tracks");
  assert.deepEqual(host.conflicts, []);
  assert.deepEqual(host.command("audio.workbench").state, { enabled: true, checked: null, busy: false, reason: null });
  assert.deepEqual(host.command("audio.reset").state, {
    enabled: false,
    checked: null,
    busy: false,
    reason: "No audio track selected",
  });
  assert.equal(host.command("audio.equalizer").state.enabled, false);
  // A disabled command's key shows its reason on the OSD and runs nothing (SDK 1.5.12, as the player).
  const replaced = replaces(host).length;
  for (const key of ["Shift+E", ">", "<", "?", "Shift+N", "Shift+C"]) {
    const shown = host.osd.length;
    assert.equal(await host.press(key), "reason", key);
    assert.deepEqual(host.osd.slice(shown), ["No audio track selected"], key);
  }
  assert.equal(replaces(host).length, replaced, "no key changed the processing");
  assert.equal(host.command("audio.sync.earlier").state.reason, "No audio track selected");
  assert.equal(host.command("audio.sync.keep.none").state.checked, true);
  assert.equal(host.command("audio.sync.keep.always").state.enabled, true, "the keep mode needs no track");
  assert.equal(host.command("audio.preset.0").state.checked, true);
  const window = host.mounted();
  assert.equal(window.props.status, "no-track");
  assert.equal(window.props["source-ready"], false);
  window.emit("delay-changed", 500); // disabled controls: ignored
  host.openElsewhere("/movie.mp4", "loading");
  await host.settle();
  assert.equal(window.props.status, "loading");
  host.setSource({ phase: "ready", audioTracks: [track(1, false)] });
  await host.settle();
  assert.equal(window.props.status, "no-track");
  host.setSource({ audioTracks: [track(1)] });
  await host.settle();
  assert.equal(window.props.status, "ready");
  assert.equal(window.props["source-ready"], true);
  assert.equal(host.command("audio.reset").state.enabled, true);
  assert.equal(replaces(host).length, 0, "a never-applied neutral chain is not sent");
  await stop();
});

test("edits round like the host: EQ ±6 dB in 0.1 dB, delay to 10 ms, crystalizer 0–6 dB", async () => {
  const { host, window, stop } = await open();
  window.emit("gain-changed", 0, 9.87);
  window.emit("gain-changed", 1, -2.345);
  window.emit("gain-changed", 10, 3); // no such band
  window.emit("gain-changed", 2, Number.NaN); // rejected, nothing changes
  window.emit("delay-changed", 104.9);
  window.emit("clarity-changed", 5); // the crystalizer is off: the slider is greyed out, nothing changes
  window.emit("clarity-toggled", true);
  window.emit("clarity-changed", 7);
  window.emit("normalize-changed", true);
  await host.settle();
  assert.deepEqual(window.props["gain-values"], [6, -2.3, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 100);
  assert.equal(window.props["clarity-db"], 6);
  assert.equal(window.props["normalize-enabled"], true);
  assert.deepEqual(own(host)?.nodes, [
    { kind: "equalizer", frequency: 31, q: 1, gainDb: 6 },
    { kind: "equalizer", frequency: 62, q: 1, gainDb: -2.3 },
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
  assert.equal(window.props.status, "applied");
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
  assert.equal(window.props.status, "applying");
  assert.equal(host.command("audio.reset").state.busy, true);
  await gate.release();
  assert.equal(own(host)?.offsetMs, 300);
  assert.ok(replaces(host).length <= 2);
  assert.equal(window.props["is-busy"], false);
  assert.equal(host.command("audio.reset").state.busy, false);
  await stop();
});

test("presets change the EQ only and the panel follows them after a drag; its Reset clears the processing and keeps the sync", async () => {
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
  assert.deepEqual(window.props["gain-values"], [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 250, "the panel's Reset keeps the sync, as Reset Audio Processing");
  assert.deepEqual(own(host)?.nodes ?? [], []);
  assert.equal(own(host)?.offsetMs, 250);
  assert.equal(host.command("audio.preset.0").state.checked, true);
  window.emit("delay-reset"); // the ↺ beside the delay
  await host.settle();
  assert.equal(own(host)?.offsetMs ?? 0, 0);
  assert.equal(window.props["delay-ms"], 0);
  window.emit("delay-changed", 40);
  await host.settle();
  await host.command("audio.reset").run();
  await host.settle();
  assert.equal(own(host)?.offsetMs, 40, "Reset Audio Processing leaves the sync alone");
  assert.equal(window.props["delay-ms"], 40);
  await host.command("audio.sync.reset").run();
  await host.settle();
  assert.equal(own(host)?.offsetMs ?? 0, 0);
  assert.equal(window.props["delay-ms"], 0);
  await stop();
});

test("Reset ▸ Audio Processing clears the equalizer, Normalize and Crystalizer, and keeps the sync and the channels", async () => {
  const host = new TestHost();
  host.stats = { ...host.stats, audio: { codec: "aac", sampleRate: 48000, channels: 6, decoder: "aac", format: "floatp", channelLayout: "5.1", bitrate: null } };
  const { window, stop } = await open(host);
  await host.settle();
  await host.command("audio.channels.left").run();
  await host.command("audio.preset.2").run();
  await host.press("Shift+E"); // equalizer bypassed, gains kept
  await host.press("Shift+N");
  await host.press("Shift+C");
  window.emit("delay-changed", 120);
  await host.settle();
  assert.equal(host.command("audio.equalizer").state.checked, false);
  assert.deepEqual((own(host)?.nodes as { kind: string }[]).map((n) => n.kind), ["channels", "treble", "dynaudnorm"]);
  await host.command("audio.reset").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Audio processing reset");
  assert.deepEqual(own(host)?.nodes, [{ kind: "channels", mode: "left" }]);
  assert.equal(own(host)?.offsetMs, 120);
  assert.equal(host.command("audio.channels.left").state.checked, true);
  assert.equal(host.command("audio.equalizer").state.checked, true);
  assert.equal(host.command("audio.normalize").state.checked, false);
  assert.equal(host.command("audio.clarity").state.checked, false);
  assert.equal(host.command("audio.preset.0").state.checked, true);
  assert.deepEqual(window.props["gain-values"], [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 120);
  await stop();
});

test("under Don't Keep another load of a source resets only the delay; an old success does not report", async () => {
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
  assert.deepEqual(latest(host).nodes, [{ kind: "equalizer", frequency: 31, q: 1, gainDb: 3 }], "the equalizer is global");
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.equal(window.props.status, "applied");
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
  assert.deepEqual(own(host)?.nodes, [{ kind: "equalizer", frequency: 31, q: 1, gainDb: 3 }]);
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.equal(window.props["delay-ms"], 500);
  assert.equal(window.props.status, "applied");
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
    assert.equal(window.props.status, "applied");
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
  assert.equal(window.props.status, "failed");
  assert.equal(window.props["error-detail"], "offset out of range");
  assert.equal(host.command("audio.reset").state.busy, false);
  host.setSource({ position: 3 }); // ordinary source updates keep the error, nothing "pending"
  await host.settle();
  assert.equal(window.props.status, "failed");
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
  assert.equal(window.props.status, "applied");
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
  assert.equal(window.props.status, "applied");
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
  assert.equal(window.props.status, "applied");
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
  assert.equal(host.osd.at(-1), "Equalizer: Off");
  assert.equal(host.command("audio.equalizer").state.checked, false);
  assert.equal(window.props["equalizer-enabled"], false);
  assert.deepEqual(window.props["gain-values"], [3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 2 }]);
  await host.press("Shift+E");
  await host.settle();
  assert.equal(host.osd.at(-1), "Equalizer: On");
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
  assert.equal(host.osd.at(-1), "Equalizer preset: Voice");
  assert.equal(window.props["equalizer-enabled"], true);
  assert.equal(window.props["preset-index"], 1);
  assert.equal(host.command("audio.preset.1").state.checked, true);
  await host.command("audio.reset").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Audio processing reset");
  assert.deepEqual(own(host)?.nodes ?? [], []);
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
  assert.equal(host.osd.at(-1), "Audio sync: -0.05 s");
  await host.press(">");
  await host.press(">");
  assert.equal(host.osd.at(-1), "Audio sync: -0.15 s");
  await host.settle();
  assert.equal(window.props["delay-ms"], -150);
  assert.equal(own(host)?.offsetMs, -150);
  for (let i = 0; i < 9; i++) await host.press("<");
  assert.equal(host.osd.at(-1), "Audio sync: +0.3 s");
  await host.press("?");
  assert.equal(host.osd.at(-1), "Audio sync: 0 s");
  await host.settle();
  assert.equal(own(host)?.offsetMs ?? 0, 0);
  window.emit("delay-changed", 9980);
  await host.press("<");
  await host.press("<");
  assert.equal(host.osd.at(-1), "Audio sync: +10 s", "the edge keeps the value and still reports it");
  await host.settle();
  assert.equal(own(host)?.offsetMs, 10000);
  window.emit("delay-reset");
  await host.settle();
  assert.equal(own(host)?.offsetMs ?? 0, 0);
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
  assert.equal(host.osd.at(-1), "Audio sync: -0.15 s");
  await gate.release();
  await Promise.all(presses);
  assert.equal(own(host)?.offsetMs, -150);
  await stop();
});

test("G09: Keep for This Run keeps the delay across sources; Always Keep also keeps it across restarts", async () => {
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

    // Keep for This Run: a restart starts at 0.
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
    assert.equal(started.host.mounted().props["delay-ms"], 0);

    // Always Keep: the delay is written and comes back.
    await started.host.command("audio.sync.keep.always").run();
    started.host.openElsewhere("/movie.mp4");
    started.host.setSource({ audioTracks: [track(1)] });
    await started.host.settle();
    await started.host.press("<");
    await started.host.press("<");
    await started.stop();
    const saved = JSON.parse(await readFile(join(data, "settings.json"), "utf8"));
    assert.equal(saved.version, 2);
    assert.equal(saved.syncKeep, "always");
    assert.equal(saved.syncMs, 100);
    const host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.equal(own(host)?.offsetMs, 100);
    await host.command("audio.workbench").run();
    const window = host.mounted();
    assert.equal(window.props["delay-ms"], 100);
    assert.equal(window.props["sync-keep-index"], 2);
    window.emit("sync-keep-selected", 0);
    await started.stop();
    assert.equal(JSON.parse(await readFile(join(data, "settings.json"), "utf8")).syncMs, undefined);
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("G11: Shift+C switches the crystalizer at the remembered strength; commands, panel and OSD agree", async () => {
  const { host, window, stop } = await open();
  assert.equal(window.props["clarity-enabled"], false);
  assert.equal(window.props["clarity-db"], 3, "default strength 3 dB");
  await host.press("Shift+C");
  await host.settle();
  assert.equal(host.osd.at(-1), "Crystalizer: On");
  assert.equal(host.command("audio.clarity").state.checked, true);
  assert.equal(window.props["clarity-enabled"], true);
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 }]);
  window.emit("clarity-changed", 4.56);
  await host.settle();
  assert.equal(window.props["clarity-db"], 4.6);
  await host.press("Shift+C");
  await host.settle();
  assert.equal(host.osd.at(-1), "Crystalizer: Off");
  assert.deepEqual(own(host)?.nodes ?? [], []);
  assert.equal(window.props["clarity-db"], 4.6, "off keeps the strength");
  window.emit("clarity-toggled", true);
  await host.settle();
  assert.deepEqual(own(host)?.nodes, [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 4.6 }]);
  window.emit("clarity-changed", 0);
  await host.settle();
  assert.equal(host.command("audio.clarity").state.checked, true, "on at 0 dB stays checked");
  assert.deepEqual(own(host)?.nodes ?? [], [], "but makes no node");
  window.emit("reset-requested"); // the panel's Reset: Crystalizer off, at the default strength
  await host.settle();
  assert.equal(window.props["clarity-enabled"], false);
  assert.equal(window.props["clarity-db"], 3);
  await host.press("Shift+N");
  await host.settle();
  assert.equal(host.osd.at(-1), "Normalize: On");
  assert.equal(window.props["normalize-enabled"], true);
  await stop();
});

test("global settings survive a restart; the old window's page is dropped", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    let host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.command("audio.preset.1").run();
    await host.press("Shift+C");
    await host.command("audio.workbench").run();
    host.mounted().emit("delay-changed", 400); // Don't Keep: not saved
    await started.stop();
    host = new TestHost();
    host.openElsewhere("/movie.mp4");
    host.setSource({ audioTracks: [track(1)] });
    started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.command("audio.workbench").run();
    const window = host.mounted();
    assert.equal(window.props["preset-index"], 1);
    assert.equal(window.props["clarity-enabled"], true);
    assert.equal(window.props["delay-ms"], 0);
    assert.equal(window.props.status, "applied", "saved settings are applied at start");
    assert.equal((own(host)?.nodes as unknown[]).length, 11, "Voice: ten bands, and the crystalizer");
    await started.stop();
    assert.equal("tab" in JSON.parse(await readFile(join(data, "settings.json"), "utf8")), false);
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

// ---------------------------------------------------------------- tracks, channels, external audio

const titled = (trackId: number, title: string, language: string | null, selected: boolean) => ({
  ...track(trackId, selected),
  title,
  language,
});

test("G03: the Audio Track menu lists the tracks, follows the list and switches with OSD; stale is silent", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [titled(1, "日语", "jpn", true), titled(2, "English", "eng", false)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  const items = () =>
    [...host.commands.values()]
      .filter(({ declaration: d }) => d.path[0] === "Audio Track" && d.order === 505)
      .map(({ display }) => display.path[1]);
  assert.deepEqual(items(), ["1. 日语 [jpn] (aac)", "2. English [eng] (aac)"]);
  assert.equal(host.command("audio.track.1").state.checked, true);
  assert.equal(host.command("audio.track.1").declaration.radio, true);
  await host.command("audio.track.2").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Audio track: 2. English [eng] (aac)");
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
  assert.deepEqual(items(), ["Loading…"]);
  assert.equal(host.command("audio.track.cycle").state.reason, "Loading");
  host.setSource({ phase: "ready", audioTracks: [] });
  await host.settle();
  assert.deepEqual(items(), ["No Audio Tracks"]);
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
  assert.deepEqual(host.osd, ["Audio track: Track 2 (aac)", "Audio track: Track 3 (aac)", "Audio track: Track 1 (aac)"]);
  assert.deepEqual(host.sent("audio.selectTrack").map((p) => p.id), [2, 3, 1]);
  host.setSource({ audioTracks: [track(1, true)] });
  await host.settle();
  await host.press("Alt+A");
  assert.equal(host.osd.at(-1), "Only one audio track");
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
  assert.equal(host.osd.at(-1), "Channels: Left Channel (mixed to stereo)");
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
  assert.equal(host.command("audio.channels.mono").state.reason, "The current track is mono");
  assert.equal(window.props["channels-available"], false);
  assert.equal(window.props["mono-track"], true);
  assert.equal(window.props["channel-index"], 4, "the setting is kept");
  host.stats = { ...host.stats, audio: { ...host.stats.audio!, channels: 2 } };
  host.setSource({ audioTracks: [track(1, false), track(2)] });
  await host.settle(20);
  assert.deepEqual((own(host)?.nodes as unknown[])[0], { kind: "channels", mode: "swap" });
  await host.command("audio.channels.stereo").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Channels: Stereo");
  assert.deepEqual((own(host)?.nodes as { kind: string }[]).map((n) => n.kind), ["equalizer"]);
  await stop();
});

test("A11: Load External Audio… adds and selects the file once; failures and a changed source are reported", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [track(1)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.command("audio.workbench").run();
  const window = host.mounted();
  assert.equal(host.command("audio.load").state.enabled, true);
  assert.equal(window.props["can-load"], true);
  host.dialogAnswers.push(null); // cancelled: nothing happens
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.sent("audio.addTrack").length, 0);
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  window.emit("load-requested");
  await host.settle();
  assert.equal(host.osd.at(-1), "Audio loaded: movie.jpn.mka");
  assert.deepEqual(host.sent("audio.addTrack").at(-1)!.path, "/music/movie.jpn.mka");
  assert.equal(host.sent("audio.addTrack").at(-1)!.select, true, "selected");
  assert.deepEqual(listed(host, window.props["track-rows"]), ["Track 1 (aac)", "2. movie.jpn.mka (external)"]);
  assert.equal(window.props["track-index"], 1);
  assert.equal(host.command("audio.track.2").state.checked, true);
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "This audio is already loaded");
  assert.equal(host.sent("audio.addTrack").length, 1);
  host.handle("audio.addTrack", () => {
    throw new HostError("backend", "unrecognized file format");
  });
  host.dialogAnswers.push(["/music/notes.txt"]);
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Cannot load audio: unrecognized file format");
  host.reset("audio.addTrack");
  // The source changes while the dialog is open.
  host.handle("dialogs.open", () => {
    host.openElsewhere("/next.mkv");
    host.setSource({ audioTracks: [track(1)] });
    return { cancelled: false, paths: ["/music/next.mp3"] };
  });
  await host.command("audio.load").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "The source changed; the audio was not loaded");
  assert.deepEqual(host.source.audioTracks?.map((t) => t.trackId), [1]);
  host.reset("dialogs.open");
  host.openElsewhere("/later.mkv", "loading");
  await host.settle();
  assert.equal(host.command("audio.load").state.reason, "Loading");
  assert.equal(window.props["can-load"], false);
  await stop();
});

test("A11: auto-loading same-named audio adds same-named audio next to a local file without selecting it", async () => {
  const folder = await mkdtemp(join(tmpdir(), "vestra-audio-media-"));
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    await mkdir(join(folder, "sub"));
    // film.Extended.ac3 names film.Extended.mkv more closely, so it is left to that video.
    const files = ["film.mkv", "film.ac3", "film.jpn.mka", "film.srt", "other.mp3", "sub/film.flac", "film.Extended.mkv", "film.Extended.ac3"];
    for (const name of files) await writeFile(join(folder, name), "");
    const host = new TestHost();
    host.openElsewhere(join(folder, "film.mkv"));
    host.setSource({ audioTracks: [track(1)] });
    let started = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.equal(host.sent("audio.addTrack").length, 0, "off by default");
    await host.command("audio.workbench").run();
    host.mounted().emit("auto-load-changed", true);
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
    // The closer video gets the file the first one left.
    next.openElsewhere(join(folder, "film.Extended.mkv"));
    next.setSource({ audioTracks: [track(1)] });
    await next.settle(20);
    assert.deepEqual(next.sent("audio.addTrack").slice(2).map((p) => p.path), [join(folder, "film.Extended.ac3")]);
    await started.stop();
  } finally {
    await rm(folder, { recursive: true, force: true });
    await rm(data, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------- G16 output device

const SPEAKERS = { id: "coreaudio/BuiltInSpeakerDevice", name: "MacBook Air 扬声器" };
const PHONES = { id: "coreaudio/USB-1", name: "USB 耳机" };
const device = (name: string, disconnected = false) => (disconnected ? `${name} (not connected)` : name);
const deviceItems = (host: TestHost) =>
  [...host.commands.entries()]
    .filter(([id, { declaration: d }]) => id.startsWith("audio.device.") && d.path.length === 0)
    .map(([id, { declaration: d, display, state }]) => [display.title, d.order, d.group, d.radio, state.enabled, state.checked, state.reason, id] as const);
const deviceCommand = (host: TestHost, title: string) => host.command(deviceItems(host).find((item) => item[0] === title)![7]);
const savedJson = async (path: string) => JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
/** The player noticed a plug or unplug (it checks every second): what `audioOutput` becomes. */
function plug(host: TestHost, devices: { id: string; name: string }[]) {
  host.audioDevices = devices;
  const requested = host.source.audioOutput?.requested ?? null;
  host.setSource({ audioOutput: { requested, active: requested !== null && devices.some((d) => d.id === requested) ? requested : null } });
}

test("G16: the palette lists System Default and the devices (no menu); choosing one switches, says so and is saved", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-audio-"));
  try {
    const host = new TestHost();
    host.audioDevices = [SPEAKERS, PHONES];
    const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data });
    await host.settle();
    assert.deepEqual(deviceItems(host).map((item) => item.slice(0, 7)), [
      ["System Default", 0, "domain", true, true, true, null],
      ["MacBook Air 扬声器", 0, "domain", true, true, false, null],
      ["USB 耳机", 0, "domain", true, true, false, null],
    ]);
    assert.equal(host.sent("player.setAudioDevice").length, 0, "System Default at start sends nothing");
    await deviceCommand(host, "USB 耳机").run();
    await host.settle();
    assert.deepEqual(host.sent("player.setAudioDevice").at(-1), { id: PHONES.id });
    assert.deepEqual(host.source.audioOutput, { requested: PHONES.id, active: PHONES.id });
    assert.equal(host.osd.at(-1), "Audio output: USB 耳机");
    assert.equal(deviceCommand(host, "USB 耳机").state.checked, true);
    assert.equal(deviceCommand(host, "System Default").state.checked, false);
    // No source is needed, and the panel shows the same choice.
    await host.command("audio.workbench").run();
    const window = host.mounted();
    assert.deepEqual(listed(host, window.props["device-rows"]), ["System Default", device(SPEAKERS.name), device(PHONES.name)]);
    assert.equal(window.props["device-index"], 2);
    assert.equal(window.props["devices-available"], true);
    assert.equal(host.translate(window.props["device-tip"] as never), PHONES.name);
    window.emit("device-selected", 0);
    await host.settle();
    assert.deepEqual(host.sent("player.setAudioDevice").at(-1), { id: null });
    assert.equal(host.osd.at(-1), "Audio output: System Default");
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
    assert.equal(host.osd.at(-1), "Cannot switch to USB 耳机: 设备被占用");
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
  assert.equal(host.osd.at(-1), "Output device disconnected, using System Default");
  // The check mark stays on the choice, now disabled and listed after the connected ones.
  assert.deepEqual(deviceItems(host).map((item) => item.slice(0, 7)), [
    ["System Default", 0, "domain", true, true, false, null],
    ["MacBook Air 扬声器", 0, "domain", true, true, false, null],
    ["USB 耳机", 0, "domain", true, false, true, "Device not connected"],
  ]);
  await host.command("audio.workbench").run();
  const window = host.mounted();
  assert.deepEqual(listed(host, window.props["device-rows"]), ["System Default", device(SPEAKERS.name), device(PHONES.name, true)]);
  assert.equal(window.props["device-index"], 2);
  assert.equal(host.translate(window.props["device-tip"] as never), `${PHONES.name}: device not connected, using System Default`);
  const sent = host.sent("player.setAudioDevice").length;
  window.emit("device-selected", 2); // the disconnected entry cannot be picked
  await host.settle();
  assert.equal(host.sent("player.setAudioDevice").length, sent);
  plug(host, [SPEAKERS, PHONES]);
  await host.settle();
  assert.equal(host.osd.at(-1), "Audio output: USB 耳机");
  assert.equal(deviceCommand(host, "USB 耳机").state.enabled, true);
  assert.deepEqual(listed(host, window.props["device-rows"]), ["System Default", device(SPEAKERS.name), device(PHONES.name)]);
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
    assert.deepEqual(deviceItems(host).at(-1)!.slice(0, 7), ["USB 耳机", 0, "domain", true, false, true, "Device not connected"]);
    assert.deepEqual(host.osd, []);
    plug(host, [SPEAKERS, PHONES]);
    await host.settle();
    assert.equal(host.osd.at(-1), "Audio output: USB 耳机", "plugging it in later is announced");
    assert.equal((await savedJson(join(data, "settings.json"))).device, PHONES.id, "the choice is never dropped");
    await started.stop();
  } finally {
    await rm(data, { recursive: true, force: true });
  }
});

test("G16: a device plugged in later shows up within 5 s; without a list the palette has none and the drop-down says so", async () => {
  const host = new TestHost();
  host.handle("player.audioDevices", () => {
    throw new HostError("unavailable", "no audio output");
  });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  assert.deepEqual(deviceItems(host), []);
  await host.command("audio.workbench").run();
  const window = host.mounted();
  assert.deepEqual(listed(host, window.props["device-rows"]), ["No output devices available"]);
  assert.equal(host.translate(window.props["device-tip"] as never), "No output devices available");
  assert.equal(window.props["devices-available"], false);
  host.reset("player.audioDevices");
  host.audioDevices = [SPEAKERS, PHONES];
  await host.settle(5200);
  assert.deepEqual(deviceItems(host).map((item) => item[0]), ["System Default", "MacBook Air 扬声器", "USB 耳机"]);
  assert.deepEqual(listed(host, window.props["device-rows"]), ["System Default", device(SPEAKERS.name), device(PHONES.name)]);
  assert.deepEqual(host.osd, [], "a new device is not announced");
  await stop();
});

// ---------------------------------------------------------------- i18n: a Simplified Chinese user sees the texts of before

test("zh-Hans: the menu, the palette names, the reasons and the on-screen messages read as before the English source", async () => {
  const host = new TestHost();
  host.audioDevices = [SPEAKERS, PHONES];
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.settle();
  // A menu item by its path, a palette-only command as the palette names it ("Category: Title").
  assert.deepEqual(
    [...host.commands.values()].map(({ display }) => (display.path.length ? display.path.join(" ▸ ") : `${display.category}: ${display.title}`)),
    [
      "打开 ▸ 载入外部音频…",
      "声音…",
      "音轨: 按序切换",
      ...["立体声", "左声道", "右声道", "单声道混合", "左右互换"].map((label) => `声道: ${label}`),
      "声音: 均衡器",
      ...["平直", "人声", "音乐", "影院"].map((label) => `均衡器预设: ${label}`),
      "声音同步: 超前 50 毫秒",
      "声音同步: 滞后 50 毫秒",
      "声音同步: 复位",
      "声音同步: 不保持",
      "声音同步: 本次运行内保持",
      "声音同步: 始终保持",
      "声音处理: 规格化",
      "声音处理: 晶化",
      "声音处理: 复位",
      "音轨 ▸ 没有音轨",
      "输出设备: 系统默认",
      "输出设备: MacBook Air 扬声器",
      "输出设备: USB 耳机",
    ],
  );
  const reason = (id: string) => host.translate(host.command(id).state.reason!);
  assert.equal(reason("audio.load"), "没有正在播放的文件");
  assert.equal(reason("audio.track.cycle"), "没有音轨");
  assert.equal(reason("audio.reset"), "没有选中的音轨");
  assert.equal(reason("audio.channels.left"), "没有选中的音轨");
  // Output device: switched, refused, unplugged (the menu says why), back.
  await deviceCommand(host, "USB 耳机").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "音频输出: USB 耳机");
  host.handle("player.setAudioDevice", () => {
    throw new HostError("busy", "设备被占用");
  });
  await deviceCommand(host, "系统默认").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "无法切换到 系统默认: 设备被占用");
  host.reset("player.setAudioDevice");
  plug(host, [SPEAKERS]);
  await host.settle();
  assert.equal(host.osd.at(-1), "输出设备已断开，改用系统默认");
  assert.equal(host.translate(host.command("audio.device.2").state.reason!), "设备未连接");
  plug(host, [SPEAKERS, PHONES]);
  await host.settle();
  assert.equal(host.osd.at(-1), "音频输出: USB 耳机");
  await deviceCommand(host, "系统默认").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "音频输出: 系统默认");
  // A source with three tracks: the processing, sync and track messages.
  host.stats = { ...host.stats, audio: { codec: "aac", sampleRate: 48000, channels: 6, decoder: "aac", format: "floatp", channelLayout: "5.1", bitrate: null } };
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [titled(1, "日语", "jpn", true), track(2, false)] });
  await host.settle(20);
  const shown: string[] = [];
  const say = async (run: () => Promise<unknown>) => {
    await run();
    await host.settle();
    shown.push(host.osd.at(-1)!);
  };
  await say(() => host.press("Shift+E"));
  await say(() => host.press("Shift+E"));
  await say(() => host.command("audio.preset.3").run());
  await say(() => host.press(">"));
  await say(() => host.press("?"));
  await say(() => host.press("Shift+N"));
  await say(() => host.press("Shift+C"));
  await say(() => host.command("audio.channels.mono").run());
  await say(() => host.command("audio.channels.stereo").run());
  await say(() => host.command("audio.reset").run());
  await say(() => host.press("Alt+A"));
  await say(() => host.press("Alt+A"));
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  await say(() => host.command("audio.load").run());
  await say(() => host.command("audio.track.1").run());
  await say(() => host.command("audio.track.3").run());
  host.dialogAnswers.push(["/music/movie.jpn.mka"]);
  await say(() => host.command("audio.load").run());
  assert.deepEqual(shown, [
    "均衡器: 关",
    "均衡器: 开",
    "均衡器预设: 影院",
    "音频同步: -0.05秒",
    "音频同步: 0秒",
    "规格化: 开",
    "晶化: 开",
    "声道: 单声道混合（已混为立体声）",
    "声道: 立体声",
    "音频处理已复位",
    "音轨: 音轨 2 (aac)",
    "音轨: 1. 日语 [jpn] (aac)",
    "已载入音频: movie.jpn.mka",
    "音轨: 1. 日语 [jpn] (aac)",
    "音轨: 3. movie.jpn.mka（外部）",
    "该音频已载入",
  ]);
  assert.deepEqual(
    ["audio.track.1", "audio.track.2", "audio.track.3"].map((id) => host.command(id).display.path.join(" ▸ ")),
    ["音轨 ▸ 1. 日语 [jpn] (aac)", "音轨 ▸ 音轨 2 (aac)", "音轨 ▸ 3. movie.jpn.mka（外部）"],
  );
  host.setSource({ audioTracks: [track(1)] });
  await host.settle();
  await host.press("Alt+A");
  assert.equal(host.osd.at(-1), "只有一条音轨");
  await stop();
});

test("zh-Hans: a track without title or codec, an external one, and unnamed devices read as before", async () => {
  const host = new TestHost();
  host.audioDevices = [{ id: "a", name: "" }, { id: "b", name: " " }, { id: "c", name: "USB 耳机" }, { id: "d", name: "USB 耳机" }];
  host.openElsewhere("/movie.mkv");
  host.setSource({ audioTracks: [{ ...track(1, true), codec: null }, { ...track(2, false), codec: "opus", external: true }] });
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  await host.settle();
  assert.deepEqual(
    ["audio.track.1", "audio.track.2"].map((id) => host.command(id).display.path.at(-1)),
    ["音轨 1", "音轨 2 (opus)（外部）"],
  );
  await host.command("audio.track.2").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "音轨: 音轨 2 (opus)（外部）");
  assert.deepEqual(
    deviceItems(host).map((item) => item[0]),
    ["系统默认", "未命名设备", "未命名设备 (2)", "USB 耳机", "USB 耳机 (2)"],
  );
  // The panel's lists arrive translated, with the words its .slint file used to put in.
  await host.command("audio.workbench").run();
  const window = host.mounted();
  assert.deepEqual(listed(host, window.props["track-rows"]), ["音轨 1", "音轨 2 (opus)（外部）"]);
  assert.deepEqual(listed(host, window.props["device-rows"]), ["系统默认", "未命名设备", "未命名设备", "USB 耳机", "USB 耳机"]);
  assert.equal(host.translate(window.props["device-tip"] as never), "系统默认");
  host.setSource({ audioTracks: [] });
  await host.settle();
  assert.deepEqual(listed(host, window.props["track-rows"]), ["没有音轨"]);
  await stop();
});

test("G03: every Audio Track menu item fits the player's menu path segment in every language", async () => {
  const awkward = [
    { title: "长".repeat(80), language: null, codec: "aac", external: false },
    { title: "日本語吹き替え版".repeat(10), language: "jpn", codec: "aac", external: true },
    { title: "x".repeat(200), language: "l".repeat(40), codec: "c".repeat(60), external: true },
    { title: "  Director's commentary \t ", language: " eng ", codec: " ac3 ", external: false },
    { title: "a\u0000b\u0007c\u001bd\u007fe\u0085f\u009fg\r\n", language: null, codec: "e\u0080ac3", external: true },
    { title: "\u0000\u0085　", language: null, codec: "opus", external: true },
    { title: "🎬👨‍👩‍👧‍👦🇯🇵".repeat(12), language: null, codec: "flac", external: true },
    { title: "é".repeat(50), language: null, codec: null, external: false },
    { title: null, language: null, codec: "\u0001" + "z".repeat(100), external: true },
    { title: null, language: null, codec: null, external: true },
    { title: "长".repeat(80), language: null, codec: "aac", external: false },
  ];
  const tracks = awkward.map((fields, index) => ({ ...track(index + 1, index === 0), ...fields }));
  const bytes = (text: string) => new TextEncoder().encode(text).length;
  const shown = new Map<string, string[]>();
  for (const locale of LOCALES) {
    const host = new TestHost();
    host.openElsewhere("/movie.mkv");
    host.setSource({ audioTracks: tracks });
    const { stop } = await startPlugin(activate, { directory, host, locale });
    try {
      await host.settle();
      const items = tracks.map(({ trackId }) => host.command(`audio.track.${trackId}`));
      for (const item of items) {
        // What the player checks (commands.rs): 1..=80 bytes, no control, no surrounding whitespace.
        for (const segment of item.display.path) {
          assert.ok(segment.length > 0 && bytes(segment) <= 80, `${locale}: ${JSON.stringify(segment)} is ${bytes(segment)} bytes`);
          assert.doesNotMatch(segment, /\p{Cc}/u, `${locale}: ${JSON.stringify(segment)}`);
          assert.doesNotMatch(segment, /^\s|\s$/u, `${locale}: ${JSON.stringify(segment)}`);
        }
      }
      const labels = items.map((item) => item.display.path[1]!);
      assert.equal(new Set(labels).size, labels.length, `${locale}: distinct items`);
      shown.set(locale, labels);
    } finally {
      await stop();
    }
  }
  // The catalogs were read: the words around the data are translated in each language.
  const english = shown.get("en")!;
  assert.ok(LOCALES.length > 1);
  for (const locale of LOCALES.slice(1)) assert.notEqual(shown.get(locale)!.at(-2), english.at(-2), `${locale}: "Track 10 (external)" is translated`);
  assert.equal(shown.get("ja")!.at(-2), "トラック 10（外部）");
  assert.match(shown.get("ja")![1]!, /^2\. 日本語吹き替え版.*… \[jpn\] \(aac\)（外部）$/);
  assert.equal(english[3], "4. Director's commentary [eng] (ac3)");
  assert.equal(english[4], "5. a b c d e f g (e ac3) (external)");
  assert.equal(english[5], "Track 6 (opus) (external)");
});

// ---------------------------------------------------------------- command palette (player 0.7.6)

/** Every command's palette category as shown, and its home, by id. */
const palette = (host: TestHost) =>
  Object.fromEntries([...host.commands.entries()].map(([id, { declaration: d, display }]) => [id, [display.category, d.home ?? null]]));

test("palette: every command has a category, in every state and language, and names the panel it lives in", async () => {
  for (const locale of LOCALES) {
    // Without tracks or devices (the track placeholder), then with both.
    const shown = async (setup: (host: TestHost) => void) => {
      const host = new TestHost();
      setup(host);
      const { stop } = await startPlugin(activate, { directory, host, locale });
      await host.settle();
      const result = palette(host);
      await stop();
      return result;
    };
    const before = await shown(() => undefined);
    const after = await shown((host) => {
      host.audioDevices = [SPEAKERS, PHONES];
      host.setSource({ audioTracks: [track(1), track(2, false)] });
    });
    for (const [id, [category]] of Object.entries({ ...before, ...after }))
      assert.ok(typeof category === "string" && category.trim() !== "", `${locale}: ${id} has no category`);
    assert.ok("audio.track.none" in before && !("audio.device.none" in before));
    assert.ok("audio.track.1" in after && "audio.track.2" in after && "audio.device.2" in after);
  }

  const host = new TestHost();
  host.audioDevices = [SPEAKERS];
  host.setSource({ audioTracks: [track(1)] });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  const home = "audio.workbench";
  assert.deepEqual(palette(host), {
    "audio.load": ["Audio", home],
    "audio.workbench": ["Audio", null],
    "audio.track.cycle": ["Audio Track", home],
    ...Object.fromEntries(["stereo", "left", "right", "mono", "swap"].map((id) => [`audio.channels.${id}`, ["Channels", home]])),
    "audio.equalizer": ["Audio", home],
    ...Object.fromEntries(presets.map((_, i) => [`audio.preset.${i}`, ["Equalizer Preset", home]])),
    "audio.sync.earlier": ["Audio Sync", home],
    "audio.sync.later": ["Audio Sync", home],
    "audio.sync.reset": ["Audio Sync", home],
    "audio.sync.keep.none": ["Audio Sync", home],
    "audio.sync.keep.session": ["Audio Sync", home],
    "audio.sync.keep.always": ["Audio Sync", home],
    "audio.normalize": ["Audio Processing", home],
    "audio.clarity": ["Audio Processing", home],
    "audio.reset": ["Audio Processing", home],
    "audio.track.1": ["Audio Track", home],
    "audio.device.0": ["Output Device", home],
    "audio.device.1": ["Output Device", home],
  });
  await stop();

  // The categories read as the menus did: the old submenu labels' translations, "Audio" the player's.
  const zh = new TestHost();
  const started = await startPlugin(activate, { directory, host: zh, locale: "zh-Hans" });
  await zh.settle();
  assert.deepEqual(
    [...new Set(Object.values(palette(zh)).map(([category]) => category))],
    ["声音", "音轨", "声道", "均衡器预设", "声音同步", "声音处理"],
  );
  await started.stop();
});

// ---------------------------------------------------------------- the Audio panel (adjust column, menu redesign step 5)

test("Audio… (Alt+2) opens the adjust column at the Audio tab, and closes it from there; the panel's commands name it as their home", async () => {
  const host = new TestHost();
  host.mountOther("adjust", "vestra.video", { title: "Picture" });
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  assert.equal(host.layout.adjust?.preferred, false);
  host.selectTab("vestra.video");
  await host.settle();
  assert.equal(await host.press("Alt+2"), "ran");
  await host.settle();
  assert.equal(host.layout.adjust?.preferred, true, "Alt+2 opens the column");
  assert.equal(host.layout.adjust?.owner, 1, "at the Audio tab");
  await host.command("audio.workbench").run();
  await host.settle();
  assert.equal(host.layout.adjust?.preferred, false, "again on the shown tab: closed");
  assert.equal(host.command("audio.workbench").display.title, "Audio…");
  for (const id of ["audio.equalizer", "audio.sync.reset", "audio.reset", "audio.track.cycle", "audio.load", "audio.channels.left"])
    assert.equal(host.command(id).declaration.home, "audio.workbench", id);
  await stop();
});

test("the panel shows each command's current key: the default, a user's own, none once cleared", async () => {
  const { host, window, stop } = await open();
  await host.settle();
  // The test host writes keys as the player does on macOS (⌥ Alt, ⇧ Shift, ⌘ Ctrl).
  assert.deepEqual(window.props.keys, { track: "⌥A", earlier: ">", later: "<", reset: "?", equalizer: "⇧E", normalize: "⇧N", clarity: "⇧C" });
  host.rebind("audio.equalizer", ["Ctrl+E"]);
  await host.settle();
  assert.equal((window.props.keys as Record<string, string>).equalizer, "⌘E");
  host.rebind("audio.sync.reset", []);
  await host.settle();
  assert.equal((window.props.keys as Record<string, string>).reset, "");
  await stop();
});

test("activated before a file is open, the panel follows the file opened later (events)", async () => {
  const { host, stop } = await startPlugin(activate, { directory });
  const panel = host.mounted();
  await host.settle();
  assert.equal(panel.props["source-ready"], false);
  host.openElsewhere("/movie.mp4");
  host.setSource({ audioTracks: [track(1)] });
  await host.settle();
  assert.equal(panel.props["source-ready"], true);
  await stop();
});

test("the panel's volume slider and mute set the player's output; −50 / +50 step the sync without on-screen text", async () => {
  const host = new TestHost();
  host.audioDevices = [SPEAKERS];
  const { window, stop } = await open(host);
  await host.settle();
  assert.equal(window.props["can-volume"], true);
  assert.equal(window.props["volume-reason"], "");
  assert.equal(window.props.volume, 100);
  assert.equal(window.props.muted, false);
  window.emit("volume-changed", 42.4);
  await host.settle();
  assert.deepEqual(host.sent("playback.set").at(-1), { property: "volume", value: 42 });
  assert.equal(window.props.volume, 42);
  host.setSource({ output: { volume: 150, muted: false } }); // louder, set elsewhere: the slider's right end
  await host.settle();
  assert.equal(window.props.volume, 100);
  window.emit("mute-toggled");
  await host.settle();
  assert.equal(window.props.muted, true);
  const shown = host.osd.length;
  window.emit("sync-stepped", -1);
  window.emit("sync-stepped", -1);
  window.emit("sync-stepped", 7); // not a step: ignored
  await host.settle();
  assert.equal(window.props["delay-ms"], -100);
  window.emit("sync-stepped", 1);
  await host.settle();
  assert.equal(own(host)?.offsetMs, -50);
  assert.equal(host.osd.length, shown, "the panel shows the value itself");
  await stop();
});

test("without an audio output the volume is greyed out and says why; nothing is sent", async () => {
  const { host, window, stop } = await open(); // the test host lists no output device
  await host.settle();
  assert.equal(window.props["can-volume"], false);
  assert.equal(host.translate(window.props["volume-reason"] as never), "No audio output");
  window.emit("volume-changed", 30);
  window.emit("mute-toggled");
  await host.settle();
  assert.equal(host.sent("playback.set").length, 0);
  await stop();
});
