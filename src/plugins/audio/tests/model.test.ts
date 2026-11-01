import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import {
  bands, clamp, companions, deviceId, deviceLabel, deviceTitles, matches, menuText, neutral, nodes, parse, presetOf, presets, SEGMENT_BYTES, serialize, syncSeconds,
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
  assert.ok(nodes({ ...neutral(), gains: [6, 6, 6, 6, 6, 6] }).every((node) => node.kind === "equalizer"));
  assert.deepEqual(nodes({ ...neutral(), clarity: { on: false, db: 5 } }), [], "crystalizer off keeps its strength but makes no node");
});

test("a bypassed equalizer drops only the bands; presetOf names the matching preset", () => {
  const value = { ...neutral(), equalizer: false, gains: [3, 0, 0, 0, 0, 0], clarity: { on: true, db: 1 } };
  assert.deepEqual(nodes(value), [{ kind: "treble", frequency: 3000, slope: 0.5, gainDb: 1 }]);
  assert.equal(presetOf(neutral()), 0);
  assert.equal(presetOf({ ...neutral(), gains: [...presets[3].gains] }), 3);
  assert.equal(presetOf(value), -1);
});

test("settings parse leniently; a legacy numeric clarity migrates to the crystalizer switch", () => {
  assert.deepEqual(parse(null), { settings: neutral(), syncKeep: "none", autoLoad: false, tab: 0, device: null, deviceName: null, extra: {} });
  assert.deepEqual(parse({ clarity: 2.5 }).settings.clarity, { on: true, db: 2.5 });
  assert.deepEqual(parse({ clarity: 0 }).settings.clarity, { on: false, db: 3 });
  assert.deepEqual(parse({ clarity: { on: true, db: 9 } }).settings.clarity, { on: true, db: 6 });
  assert.deepEqual(parse({ gains: [1, "x", 9, 0, 0, 0] }).settings.gains, [1, 0, 6, 0, 0, 0]);
  assert.deepEqual(parse({ gains: [1, 2] }).settings.gains, [0, 0, 0, 0, 0, 0]);
  assert.equal(parse({ syncKeep: "session", syncMs: 500 }).settings.delayMs, 0, "only Always Keep restores the delay");
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
  assert.equal(syncSeconds(-50), "-0.05");
  assert.equal(syncSeconds(300), "+0.3");
  assert.equal(syncSeconds(0), "0");
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

/** The track menu templates in English and every catalog of the plugin. */
const templates = (() => {
  const ids = ["{} (external)", "Track {}", "Track {} ({})", "Track {} (external)", "Track {} ({}) (external)"];
  const catalogs = readdirSync(new URL("../locales/", import.meta.url)).filter((name) => name.endsWith(".po"));
  return [
    new Map(ids.map((id) => [id, id])),
    ...catalogs.map((name) => {
      const po = readFileSync(new URL(`../locales/${name}`, import.meta.url), "utf8");
      const pairs = [...po.matchAll(/^msgid "(.*)"\nmsgstr "(.*)"$/gm)].map(([, id, str]) => [JSON.parse(`"${id}"`), JSON.parse(`"${str}"`)] as const);
      const map = new Map(pairs);
      for (const id of ids) assert.ok(map.get(id), `${name} translates ${id}`);
      return map;
    }),
  ];
})();
const fill = (template: string, args: (string | number)[]) => args.reduce<string>((out, arg) => out.replace("{}", String(arg)), template);
/** The menu segments `trackLabel` (main.ts) makes of a row, in every language. */
const menuLabels = (row: ReturnType<typeof trackRow>) =>
  templates.map((catalog) => {
    const label = row.text ? row.text : row.codec ? fill(catalog.get("Track {} ({})")!, [row.n, row.codec]) : fill(catalog.get("Track {}")!, [row.n]);
    return row.external ? fill(catalog.get("{} (external)")!, [label]) : label;
  });
const bytes = (text: string) => new TextEncoder().encode(text).length;
const segmentValid = (text: string) => text !== "" && bytes(text) <= SEGMENT_BYTES && menuText(text) === text && !/\p{Cc}/u.test(text);

test("G03 track labels fit a menu path segment (80 bytes, no controls or edge spaces) in every language", () => {
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
      for (const label of menuLabels(row)) assert.ok(segmentValid(label), `${JSON.stringify(label)} (${bytes(label)} bytes)`);
      // A cut never splits a user-perceived character: every grapheme of the text is one of the source's, or "…".
      const source = [fields.title, fields.language, fields.codec].join(" ");
      for (const { segment } of segmenter.segment(row.text)) assert.ok(segment === "…" || /^[\d. [\]()]$/.test(segment) || source.includes(segment), segment);
    }
  }
  // A long CJK title is cut in bytes, keeping number, language and codec.
  const cjk = trackRow(t({ title: "日本語吹き替え版".repeat(10), language: "jpn", codec: "aac", external: true }), 0).text;
  assert.match(cjk, /^1\. 日本語吹き替え版.*… \[jpn\] \(aac\)$/);
  assert.equal(menuLabels(trackRow(t({ title: "日本語吹き替え版".repeat(10), language: "jpn", codec: "aac", external: true }), 0))[0], `${cjk} (external)`);
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
  // Every label of a list stays distinct, also when the titles are the same once cut.
  const list = [
    ...Array.from({ length: 12 }, () => t({ title: "长".repeat(80), codec: "aac" })),
    t({}), t({ codec: "aac" }), t({ external: true }), t({ title: "Track 2" }),
  ];
  for (const catalog of templates.keys()) {
    const labels = list.map((track, index) => menuLabels(trackRow(track, index))[catalog]);
    assert.equal(new Set(labels).size, labels.length, labels.join(" | "));
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
