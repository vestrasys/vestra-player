import assert from "node:assert/strict";
import test from "node:test";
import {
  bands, clamp, companions, deviceId, deviceLabel, deviceTitles, matches, narrow, neutral, nodes, OLD_BANDS, parse, presetOf, presets, serialize, spread, syncSeconds,
  TRACK_TEXT_BYTES, trackRow,
} from "../model.ts";

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

test("normalization and the crystalizer use typed processing nodes without device volume", () => {
  assert.deepEqual(nodes({ ...neutral(), normalize: true, clarity: { on: true, db: 3 } }), [
    { kind: "treble", frequency: 3000, slope: 0.5, gainDb: 3 },
    { kind: "dynaudnorm", enabled: true, maxGain: 4, peak: 0.9 },
  ]);
  assert.ok(nodes({ ...neutral(), gains: bands.map(() => 6) }).every((node) => node.kind === "equalizer"));
  assert.deepEqual(nodes({ ...neutral(), clarity: { on: false, db: 5 } }), [], "crystalizer off keeps its strength but makes no node");
});

test("a bypassed equalizer drops only the bands; presetOf names the matching preset", () => {
  const value = { ...neutral(), equalizer: false, gains: [3, 0, 0, 0, 0, 0, 0, 0, 0, 0], clarity: { on: true, db: 1 } };
  assert.deepEqual(nodes(value), [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 }]);
  assert.equal(presetOf(neutral()), 0);
  assert.equal(presetOf({ ...neutral(), gains: [...presets[3].gains] }), 3);
  assert.equal(presetOf(value), -1);
});

test("settings parse leniently; a legacy numeric clarity migrates to the crystalizer switch", () => {
  assert.deepEqual(parse(null), { settings: neutral(), syncKeep: "none", autoLoad: false, device: null, deviceName: null, extra: {} });
  assert.deepEqual(parse({ clarity: 2.5 }).settings.clarity, { on: true, db: 2.5 });
  assert.deepEqual(parse({ clarity: 0 }).settings.clarity, { on: false, db: 3 });
  assert.deepEqual(parse({ clarity: { on: true, db: 9 } }).settings.clarity, { on: true, db: 6 });
  assert.deepEqual(parse({ gains10: [1, "x", 9, 0, 0, 0, 0, 0, 0, -2] }).settings.gains, [1, 0, 6, 0, 0, 0, 0, 0, 0, -2]);
  assert.deepEqual(parse({ gains: [1, 2] }).settings.gains, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  assert.deepEqual(parse({ gains: [1, "x", 9, 0, 0, 0] }).settings.gains, spread([1, 0, 6, 0, 0, 0]), "six gains (version 1) are spread");
  assert.equal(parse({ syncKeep: "session", syncMs: 500 }).settings.delayMs, 0, "only Always Keep restores the delay");
  assert.equal(parse({ syncKeep: "always", syncMs: 5003 }).settings.delayMs, 5000);
  assert.equal(parse({ syncKeep: "always", syncMs: -25000 }).settings.delayMs, -10000, "G09: ±10 000 ms");
  assert.equal(parse({ syncKeep: "bogus" }).syncKeep, "none");
  assert.equal("tab" in parse({ tab: 1 }).extra, false, "the old window's page is dropped");
  assert.equal(parse({ channels: { mode: "sideways" } }).settings.channels, "stereo");
  const stored = parse({ version: 1, future: "usb", channels: { mode: "left" }, autoLoadExternal: true, syncKeep: "always", syncMs: 120 });
  assert.deepEqual(stored.extra, { future: "usb" }, "later fields are kept");
  assert.equal(stored.settings.channels, "left");
  assert.equal(stored.autoLoad, true);
  assert.deepEqual(serialize(stored), {
    future: "usb",
    version: 2,
    equalizer: true,
    gains: [0, 0, 0, 0, 0, 0],
    gains10: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    normalize: false,
    clarity: { on: false, db: 3 },
    channels: { mode: "left" },
    autoLoadExternal: true,
    syncKeep: "always",
    syncMs: 120,
    device: null,
  });
  assert.equal("syncMs" in serialize({ ...stored, syncKeep: "session" }), false);
  assert.equal(syncSeconds(-50), "-0.05");
  assert.equal(syncSeconds(300), "+0.3");
  assert.equal(syncSeconds(0), "0");
});

test("G05: the channels node comes first, only off stereo and never on a mono track", () => {
  const value = { ...neutral(), channels: "swap" as const, gains: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0], normalize: true };
  assert.deepEqual(
    nodes(value).map((n) => n.kind),
    ["channels", "equalizer", "dynaudnorm"],
  );
  assert.deepEqual(nodes(value)[0], { kind: "channels", mode: "swap" });
  assert.deepEqual(nodes(value, { mono: true }).map((n) => n.kind), ["equalizer", "dynaudnorm"]);
  assert.deepEqual(nodes({ ...neutral(), channels: "stereo" }), []);
});

test("G03 track labels: number, title, language, codec, external", () => {
  const t = (fields: Partial<Parameters<typeof trackRow>[0]>) => ({
    id: 1, selected: false, external: false, exportable: true, codec: null, title: null, language: null, ...fields,
  });
  const label = (fields: Partial<Parameters<typeof trackRow>[0]>, index: number) => trackRow(t(fields), index).text;
  assert.equal(label({ title: "日语", language: "jpn", codec: "aac" }, 0), "1. 日语 [jpn] (aac)");
  assert.equal(label({ title: "movie.ac3", codec: "ac3", external: true }, 2), "3. movie.ac3 (ac3)");
  assert.equal(label({ language: "eng" }, 0), "1. [eng]");
  // Without title and language the window, the menu and the OSD put the words around the data.
  assert.deepEqual(trackRow(t({ codec: "opus", external: true }), 1), {
    kind: "track", text: "", n: 2, codec: "opus", external: true, disconnected: false,
  });
  assert.deepEqual(trackRow(t({}), 3), { kind: "track", text: "", n: 4, codec: "", external: false, disconnected: false });
});

const bytes = (text: string) => new TextEncoder().encode(text).length;

test("G03 track labels: at most 68 bytes, cut on whole characters, no controls or edge spaces", () => {
  const t = (fields: Partial<Parameters<typeof trackRow>[0]>) => ({
    id: 1, selected: false, external: false, exportable: true, codec: null, title: null, language: null, ...fields,
  });
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  const cases: Partial<Parameters<typeof trackRow>[0]>[] = [
    { title: "长".repeat(80) },
    { title: "日本語吹き替え版".repeat(10), language: "jpn", codec: "aac", external: true },
    { title: "x".repeat(200), language: "l".repeat(40), codec: "c".repeat(60), external: true },
    { title: "  Director's commentary \t ", language: " eng ", codec: " ac3 " },
    { title: "a\u0000b\u0007c\u001bd\u007fe\u0085f\u009fg\r\n", codec: "e\u0080ac3" },
    { title: "\u0000\u0085　", codec: "opus", external: true },
    { title: "🎬👨‍👩‍👧‍👦🇯🇵".repeat(12), codec: "flac" },
    { title: "é".repeat(50) },
    { codec: "\u0001" + "z".repeat(100), external: true },
  ];
  for (const fields of cases) {
    for (const index of [0, 98_765]) {
      const row = trackRow(t(fields), index);
      assert.ok(bytes(row.text) <= TRACK_TEXT_BYTES, row.text);
      assert.doesNotMatch(row.text, /\p{Cc}/u);
      assert.doesNotMatch(row.text, /^\s|\s$/u);
      // A cut never splits a user-perceived character: every grapheme of the text is one of the source's, or "…".
      const source = [fields.title, fields.language, fields.codec].join(" ");
      for (const { segment } of segmenter.segment(row.text)) assert.ok(segment === "…" || /^[\d. [\]()]$/.test(segment) || source.includes(segment), segment);
    }
  }
  // A long CJK title is cut in bytes, keeping number, language and codec.
  const cjk = trackRow(t({ title: "日本語吹き替え版".repeat(10), language: "jpn", codec: "aac", external: true }), 0).text;
  assert.match(cjk, /^1\. 日本語吹き替え版.*… \[jpn\] \(aac\)$/);
  // Edge spaces go, controls become spaces.
  assert.equal(trackRow(t({ title: "  Director's commentary \t ", language: " eng ", codec: " ac3 " }), 0).text, "1. Director's commentary [eng] (ac3)");
  assert.equal(trackRow(t({ title: "a\u0000b\u0085c\r\n", codec: "e\u0080ac3" }), 0).text, "1. a b c (e ac3)");
  // A title of controls and spaces only is no title: the words "Track 1 (opus)" are shown.
  assert.deepEqual(trackRow(t({ title: "\u0000\u0085　", codec: "opus", external: true }), 0), {
    kind: "track", text: "", n: 1, codec: "opus", external: true, disconnected: false,
  });
  // Emoji sequences are kept or dropped whole.
  const emoji = trackRow(t({ title: "🎬👨‍👩‍👧‍👦🇯🇵".repeat(12), codec: "flac" }), 0).text;
  assert.ok(emoji.endsWith("… (flac)") && bytes(emoji) <= TRACK_TEXT_BYTES, emoji);
  assert.ok(!emoji.includes("‍…") && !/[\ud800-\udbff](?![\udc00-\udfff])/.test(emoji));
});

test("without Intl.Segmenter the labels are cut on code points and still fit", async () => {
  const saved = Object.getOwnPropertyDescriptor(Intl, "Segmenter")!;
  Object.defineProperty(Intl, "Segmenter", { value: undefined, configurable: true, writable: true });
  try {
    // A fresh copy of the module, loaded while the runtime has no segmenter.
    const model: typeof import("../model.ts") = await import(new URL("../model.ts?no-segmenter", import.meta.url).href); // boundary-reviewed: the package's own model.ts, reloaded
    const t = (title: string) => ({ id: 1, selected: false, external: false, exportable: true, codec: "aac", title, language: null });
    const text = model.trackRow(t("é".repeat(40)), 0).text;
    assert.equal(text, `1. ${"é".repeat(18)}e… (aac)`, "code points: the last accent is cut off");
    assert.ok(bytes(text) <= TRACK_TEXT_BYTES);
    assert.equal(trackRow(t("é".repeat(40)), 0).text, `1. ${"é".repeat(18)}… (aac)`, "graphemes: kept whole");
  } finally {
    Object.defineProperty(Intl, "Segmenter", saved);
  }
});

test("A11 same-name scan: audio files that start with the video's name, at most 8", () => {
  const names = ["movie.mkv", "movie.ac3", "movie.jpn.mka", "Movie.ENG.FLAC", "movie2.ac3", "movie.srt", "other.mp3", "movie"];
  assert.deepEqual(companions("movie.mkv", names), ["Movie.ENG.FLAC", "movie.ac3", "movie.jpn.mka"]);
  assert.deepEqual(companions("song.mp3", ["song.mp3", "song.flac"]), [], "an audio source loads nothing");
  const many = Array.from({ length: 12 }, (_, i) => `a.${String(i).padStart(2, "0")}.mp3`);
  assert.equal(companions("a.mp4", many).length, 8);
});

test("A11 same-name scan: names compare in NFC; a name with an empty tag (`movie..ac3`) is not the video's", () => {
  // macOS lists decomposed names; the open path may be composed, or the other way round.
  assert.deepEqual(companions("Café.mkv", ["Café.ac3", "Café.jpn.mka"]), ["Café.ac3", "Café.jpn.mka"]);
  assert.deepEqual(companions("Café.mkv", ["Café.flac"]), ["Café.flac"]);
  // The closer-video rule compares in NFC too.
  assert.deepEqual(companions("Café.mkv", ["Café.Extended.mkv", "Café.Extended.ac3", "Café.ac3"]), ["Café.ac3"]);
  assert.deepEqual(companions("movie.mkv", ["movie..ac3", "movie.ac3", "movie..jpn.mka", "movie.jpn.mka"]), [
    "movie..jpn.mka", "movie.ac3", "movie.jpn.mka",
  ]);
});

test("A11 same-name scan: a file that names another video of the folder more closely belongs to that one", () => {
  const folder = ["Movie.mkv", "Movie.Extended.mkv", "Movie.ac3", "Movie.Extended.ac3", "Movie.Extended.jpn.mka", "Movie.jpn.mka"];
  assert.deepEqual(companions("Movie.mkv", folder), ["Movie.ac3", "Movie.jpn.mka"]);
  assert.deepEqual(companions("Movie.Extended.mkv", folder), ["Movie.Extended.ac3", "Movie.Extended.jpn.mka"]);
  // Case-insensitive like the match itself; the longer video only takes what it names.
  assert.deepEqual(companions("movie.mp4", ["movie.mp4", "MOVIE.EXTENDED.MP4", "movie.extended.flac", "movie.ext.flac"]), ["movie.ext.flac"]);
  // Only videos count: an audio file, a subtitle or another file with a longer name takes nothing.
  assert.deepEqual(companions("movie.mkv", ["movie.jpn.flac", "movie.jpn.srt", "movie.jpn.nfo", "movie.jpn.ac3"]), [
    "movie.jpn.ac3", "movie.jpn.flac",
  ]);
  // A video that only shares a prefix without the dot (`Movie2`) or the same stem (`Movie.mp4`) takes nothing.
  assert.deepEqual(companions("Movie.mkv", ["Movie2.mkv", "Movie.mp4", "Movie.ac3", "Movie2.ac3"]), ["Movie.ac3"]);
});

test("G16: the saved device is an id the player accepts, kept with its name; titles stay distinct", () => {
  const stored = parse({ device: "coreaudio/AB-12", deviceName: "AirPods Pro" });
  assert.equal(stored.device, "coreaudio/AB-12");
  assert.equal(stored.deviceName, "AirPods Pro");
  assert.deepEqual(serialize(stored).device, "coreaudio/AB-12");
  assert.equal(serialize(stored).deviceName, "AirPods Pro");
  assert.equal(serialize(parse({})).device, null, "System Default is saved as null");
  assert.equal("deviceName" in serialize(parse({})), false);
  for (const bad of ["", 7, "a\nb", "x".repeat(513)]) assert.equal(parse({ device: bad, deviceName: "n" }).device, null);
  assert.equal(deviceId("coreaudio/X"), "coreaudio/X");
  // As the player checks it: at most 512 UTF-8 bytes, no C0, DEL or C1 control.
  assert.equal(deviceId("x".repeat(512)), "x".repeat(512));
  assert.equal(deviceId("扬".repeat(170)), "扬".repeat(170), "510 bytes");
  assert.equal(deviceId("扬".repeat(171)), null, "171 characters, 513 bytes");
  for (const bad of ["a\u007fb", "a\u0085b", "a\u009fb", "a\u0000"]) assert.equal(deviceId(bad), null, JSON.stringify(bad));
  assert.deepEqual(
    deviceTitles([{ id: "a", name: "USB Audio" }, { id: "b", name: "USB Audio" }, { id: "c", name: "System Default" }, { id: "d", name: " \t " }, { id: "e", name: "" }], "USB Audio"),
    [
      { name: "USB Audio", n: null, key: "USB Audio" },
      { name: "USB Audio", n: 2, key: "USB Audio (2)" },
      { name: "System Default", n: 2, key: "System Default (2)" },
      { name: "", n: null, key: "Unnamed device" },
      { name: "", n: 2, key: "Unnamed device (2)" },
      { name: "USB Audio", n: 3, key: "USB Audio (3)" },
    ],
  );
  const long = deviceLabel("扬声器".repeat(40));
  assert.ok(new TextEncoder().encode(long).length <= 72 && long.endsWith("…"));
  assert.equal(deviceLabel(" Speakers\u0085(USB)\u009b "), "Speakers (USB)", "C1 controls become spaces too");
});

test("ten bands: six saved gains follow the old curve, the ends held, and the old presets stay presets", () => {
  assert.deepEqual([...bands], [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]);
  assert.deepEqual(spread([1, 2, 3, 4, 5, 6]).slice(0, 2), [1, 1], "below 60 Hz: the first gain");
  assert.equal(spread([1, 2, 3, 4, 5, 6])[5], 4, "1 kHz is an old band");
  assert.equal(spread([1, 2, 3, 4, 5, 6])[9], 6, "above 10 kHz: the last gain");
  assert.equal(OLD_BANDS.length, 6);
  // The presets of 1.5.10, as settings.json held them, read as the same presets.
  const old = [[0, 0, 0, 0, 0, 0], [-3, -2, 1, 3, 2, -1], [3, 2, -1, 0, 2, 3], [4, 2, -1, 1, 2, 2]];
  old.forEach((gains, index) => assert.equal(presetOf(parse({ version: 1, gains }).settings), index));
  assert.equal(nodes({ ...neutral(), gains: [...presets[2].gains] }).length, 9, "Music: nine bands off 0 dB");
});

test("downgrade: the file keeps the ten gains in gains10 and a close six-band curve in gains", () => {
  const curve = [5, 4, 2, -1, -3, -2, 0, 2, 4, 6];
  const saved = serialize({ ...parse({}), settings: { ...neutral(), gains: curve } });
  assert.deepEqual(saved.gains10, curve);
  assert.equal(saved.gains.length, 6);
  // The six old bands read the ten-band curve where they sit (60 Hz lies between 31 and 62 Hz, 1 kHz on a band).
  assert.equal(saved.gains[3], -2, "1 kHz");
  assert.deepEqual(parse(JSON.parse(JSON.stringify(saved))).settings.gains, curve, "this version reads the ten gains back");
  // An older plugin changed the curve since: it wrote gains and kept gains10 as it was; its curve wins.
  const edited = { ...JSON.parse(JSON.stringify(saved)), gains: [0, 0, 0, 0, 0, 3] };
  assert.deepEqual(parse(edited).settings.gains, spread([0, 0, 0, 0, 0, 3]));
  // The presets survive both ways.
  for (const [index, preset] of presets.entries()) {
    assert.equal(presetOf(parse(spreadBack(preset.gains)).settings), index, preset.label);
  }
  assert.deepEqual(narrow(bands.map(() => 0)), [0, 0, 0, 0, 0, 0]);
});
/** A file written by this version for the given ten gains, read back as JSON. */
const spreadBack = (gains: readonly number[]) => JSON.parse(JSON.stringify(serialize({ ...parse({}), settings: { ...neutral(), gains: [...gains] } })));

test("downgrade: 1.5.10's model.ts (origin/main before the panel) reads the new file as a close, non-flat curve", async (t) => {
  const { execFileSync } = await import("node:child_process");
  const { mkdtemp, writeFile, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { pathToFileURL } = await import("node:url");
  let source: string;
  try {
    // 5b5f785a: the last commit with the six-band model (1.5.10).
    source = execFileSync("git", ["show", "5b5f785a:src/plugins/audio/model.ts"], { cwd: import.meta.dirname, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    t.skip("no git history here (shallow clone)");
    return;
  }
  const dir = await mkdtemp(join(tmpdir(), "audio-old-model-"));
  try {
    const file = join(dir, "model.ts");
    await writeFile(file, source);
    const old = (await import(pathToFileURL(file).href)) as { parse: (raw: unknown) => { settings: { gains: number[] } } };
    const curve = [5, 4, 2, -1, -3, -2, 0, 2, 4, 6];
    const saved = spreadBack(curve);
    const read = old.parse(saved).settings.gains;
    assert.deepEqual(read, narrow(curve));
    assert.ok(read.some((gain) => gain !== 0), "not flat");
    // The old plugin saves the six gains (and keeps gains10): this version reads its curve.
    assert.deepEqual(parse({ ...saved, gains: read }).settings.gains, curve, "unchanged by the old plugin: the ten gains");
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
