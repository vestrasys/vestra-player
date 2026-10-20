import assert from "node:assert/strict";
import test from "node:test";
import { bands, clamp, companions, deviceId, deviceLabel, deviceTitles, matches, neutral, nodes, parse, presetOf, presets, serialize, syncText, trackLabel } from "../model.ts";

test("reset clears the owned chain and preset gains stay inside the ±6 dB EQ range", () => {
  assert.deepEqual(nodes(neutral()), []);
  for (const preset of presets) {
    assert.equal(preset.gains.length, bands.length);
    for (const gain of preset.gains) assert.ok(Math.abs(gain) <= 6);
  }
  assert.throws(() => clamp(Number.NaN, -6, 6));
  assert.throws(() => clamp("4", -6, 6));
  assert.equal(clamp(50, -6, 6), 6);
  assert.ok(matches(neutral(), 0));
  assert.ok(!matches(neutral(), 1));
});

test("normalization and 晶化 use typed processing nodes without device volume", () => {
  assert.deepEqual(nodes({ ...neutral(), normalize: true, clarity: { on: true, db: 3 } }), [
    { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 },
    { kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 },
  ]);
  assert.ok(nodes({ ...neutral(), gains: [6, 6, 6, 6, 6, 6] }).every((node) => node.kind === "equalizer"));
  assert.deepEqual(nodes({ ...neutral(), clarity: { on: false, db: 5 } }), [], "晶化 off keeps its strength but makes no node");
});

test("a bypassed equalizer drops only the bands; presetOf names the matching preset", () => {
  const value = { ...neutral(), equalizer: false, gains: [3, 0, 0, 0, 0, 0], clarity: { on: true, db: 1 } };
  assert.deepEqual(nodes(value), [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 }]);
  assert.equal(presetOf(neutral()), 0);
  assert.equal(presetOf({ ...neutral(), gains: [...presets[3].gains] }), 3);
  assert.equal(presetOf(value), -1);
});

test("settings parse leniently; a legacy numeric clarity migrates to the 晶化 switch", () => {
  assert.deepEqual(parse(null), { settings: neutral(), syncKeep: "none", autoLoad: false, tab: 0, device: null, deviceName: null, extra: {} });
  assert.deepEqual(parse({ clarity: 2.5 }).settings.clarity, { on: true, db: 2.5 });
  assert.deepEqual(parse({ clarity: 0 }).settings.clarity, { on: false, db: 3 });
  assert.deepEqual(parse({ clarity: { on: true, db: 9 } }).settings.clarity, { on: true, db: 6 });
  assert.deepEqual(parse({ gains: [1, "x", 9, 0, 0, 0] }).settings.gains, [1, 0, 6, 0, 0, 0]);
  assert.deepEqual(parse({ gains: [1, 2] }).settings.gains, [0, 0, 0, 0, 0, 0]);
  assert.equal(parse({ syncKeep: "session", syncMs: 500 }).settings.delayMs, 0, "only 始终保持 restores the delay");
  assert.equal(parse({ syncKeep: "always", syncMs: 5003 }).settings.delayMs, 5000);
  assert.equal(parse({ syncKeep: "always", syncMs: -25000 }).settings.delayMs, -10000, "G09: ±10 000 ms");
  assert.equal(parse({ syncKeep: "bogus" }).syncKeep, "none");
  assert.equal(parse({ tab: 7 }).tab, 0);
  assert.equal(parse({ channels: { mode: "sideways" } }).settings.channels, "stereo");
  const stored = parse({ version: 1, future: "usb", channels: { mode: "left" }, autoLoadExternal: true, syncKeep: "always", syncMs: 120 });
  assert.deepEqual(stored.extra, { future: "usb" }, "later fields are kept");
  assert.equal(stored.settings.channels, "left");
  assert.equal(stored.autoLoad, true);
  assert.deepEqual(serialize(stored), {
    future: "usb",
    version: 1,
    equalizer: true,
    gains: [0, 0, 0, 0, 0, 0],
    normalize: false,
    clarity: { on: false, db: 3 },
    channels: { mode: "left" },
    autoLoadExternal: true,
    syncKeep: "always",
    syncMs: 120,
    tab: 0,
    device: null,
  });
  assert.equal("syncMs" in serialize({ ...stored, syncKeep: "session" }), false);
  assert.equal(syncText(-50), "音频同步: -0.05秒");
  assert.equal(syncText(300), "音频同步: +0.3秒");
  assert.equal(syncText(0), "音频同步: 0秒");
});

test("G05: the channels node comes first, only off stereo and never on a mono track", () => {
  const value = { ...neutral(), channels: "swap" as const, gains: [2, 0, 0, 0, 0, 0], normalize: true };
  assert.deepEqual(
    nodes(value).map((n) => n.kind),
    ["channels", "equalizer", "dynaudnorm"],
  );
  assert.deepEqual(nodes(value)[0], { kind: "channels", mode: "swap" });
  assert.deepEqual(nodes(value, { mono: true }).map((n) => n.kind), ["equalizer", "dynaudnorm"]);
  assert.deepEqual(nodes({ ...neutral(), channels: "stereo" }), []);
});

test("G03 track labels: number, title, language, codec, external", () => {
  const t = (fields: Partial<Parameters<typeof trackLabel>[0]>) => ({
    id: 1, selected: false, external: false, exportable: true, codec: null, title: null, language: null, ...fields,
  });
  assert.equal(trackLabel(t({ title: "日语", language: "jpn", codec: "aac" }), 0), "1. 日语 [jpn] (aac)");
  assert.equal(trackLabel(t({ title: "movie.ac3", codec: "ac3", external: true }), 2), "3. movie.ac3 (ac3)（外部）");
  assert.equal(trackLabel(t({ codec: "opus" }), 1), "音轨 2 (opus)");
  assert.equal(trackLabel(t({ language: "eng" }), 0), "1. [eng]");
  assert.equal([...trackLabel(t({ title: "长".repeat(80) }), 0)].length, 60);
});

test("A11 same-name scan: audio files that start with the video's name, at most 8", () => {
  const names = ["movie.mkv", "movie.ac3", "movie.jpn.mka", "Movie.ENG.FLAC", "movie2.ac3", "movie.srt", "other.mp3", "movie"];
  assert.deepEqual(companions("movie.mkv", names), ["Movie.ENG.FLAC", "movie.ac3", "movie.jpn.mka"]);
  assert.deepEqual(companions("song.mp3", ["song.mp3", "song.flac"]), [], "an audio source loads nothing");
  const many = Array.from({ length: 12 }, (_, i) => `a.${String(i).padStart(2, "0")}.mp3`);
  assert.equal(companions("a.mp4", many).length, 8);
});

test("G16: the saved device is an id the player accepts, kept with its name; titles stay distinct", () => {
  const stored = parse({ device: "coreaudio/AB-12", deviceName: "AirPods Pro" });
  assert.equal(stored.device, "coreaudio/AB-12");
  assert.equal(stored.deviceName, "AirPods Pro");
  assert.deepEqual(serialize(stored).device, "coreaudio/AB-12");
  assert.equal(serialize(stored).deviceName, "AirPods Pro");
  assert.equal(serialize(parse({})).device, null, "系统默认 is saved as null");
  assert.equal("deviceName" in serialize(parse({})), false);
  for (const bad of ["", 7, "a\nb", "x".repeat(513)]) assert.equal(parse({ device: bad, deviceName: "n" }).device, null);
  assert.equal(deviceId("coreaudio/X"), "coreaudio/X");
  assert.deepEqual(
    deviceTitles([{ id: "a", name: "USB Audio" }, { id: "b", name: "USB Audio" }, { id: "c", name: "系统默认" }, { id: "d", name: " \t " }], "USB Audio"),
    ["USB Audio", "USB Audio (2)", "系统默认 (2)", "未命名设备", "USB Audio (3)"],
  );
  const long = deviceLabel("扬声器".repeat(40));
  assert.ok(new TextEncoder().encode(long).length <= 72 && long.endsWith("…"));
});
