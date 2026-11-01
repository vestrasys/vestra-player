// Track names as menu path segments: whole characters, no controls, no edge spaces, ≤ 80 bytes, distinct.
import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { bytes, menuCut, menuText, segmentValid } from "../model.ts";

const directory = new URL("..", import.meta.url);
const LOCALES = ["en", "de", "es", "fr", "ja", "ko", "zh-Hans"] as const;
type Track = { id: number; title: string; language: string | null };
const lone = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

/** Starts the plugin with these embedded tracks and returns the menu items' segments (rendered). */
async function segments(tracks: Track[], locale: (typeof LOCALES)[number]) {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  for (const { id, title, language } of tracks)
    host.subtitles.tracks.push({ target: { kind: "embedded", id }, title, language, styleSupported: true, styleReason: null, text: true });
  const { stop } = await startPlugin(activate, { directory, host, locale });
  await host.settle();
  const shown = tracks.slice(0, 8).map((_, i) => host.command(`track-${i + 1}`).display.path.at(-1) as string);
  const second = tracks.slice(0, 8).map((_, i) => host.command(`second-${i + 1}`).display.path.at(-1) as string);
  await stop();
  return { shown, second };
}
async function check(tracks: Track[]) {
  const result: Record<string, string[]> = {};
  for (const locale of LOCALES) {
    const { shown, second } = await segments(tracks, locale);
    assert.deepEqual(second, shown, locale);
    for (const s of shown) {
      assert.ok(segmentValid(s), `${locale}: ${JSON.stringify(s)} (${bytes(s)} bytes)`);
      assert.ok(!lone.test(s), `${locale}: ${JSON.stringify(s)} splits a surrogate pair`);
    }
    assert.equal(new Set(shown).size, shown.length, `${locale}: distinct ${JSON.stringify(shown)}`);
    result[locale] = shown;
  }
  return result;
}

test("menuCut: whole graphemes, C0/DEL/C1 removed, edges trimmed after the cut", () => {
  assert.equal(menuCut("Director's cut commentary", 15, 80), "Director's cut", "a space at the cut is trimmed");
  assert.equal(menuCut("abc😀def", 4, 80), "abc😀");
  assert.equal(menuCut("abc😀def", 80, 6), "abc", "an emoji that does not fit is dropped whole");
  assert.equal(menuCut("👨‍👩‍👧‍👦 family", 1, 80), "👨‍👩‍👧‍👦", "an emoji sequence is one character");
  assert.equal(menuCut("étude", 1, 80), "é", "a combining accent stays with its letter");
  assert.equal(menuCut("\u0085a\u0080b\u009fc\u0007d\u007f", 80, 80), "a b c d");
  assert.equal(menuCut(" 　 x  ", 80, 80), "x", "Unicode White_Space at the edges");
  assert.equal(menuCut("\u0085\u0009 ", 80, 80), "");
  assert.equal(menuCut("字".repeat(40), 24, 70), "字".repeat(23), "70 bytes: 23 CJK characters");
  assert.equal(menuText("a\u009bb"), "a b");
});

test("track titles that end in a space at the cut are trimmed", async () => {
  const shown = await check([{ id: 1, title: "Commentary with the director", language: null }]);
  assert.equal(shown.en![0], "Commentary with the dire");
  const spaced = await check([{ id: 1, title: "Twenty-three characters text", language: null }]);
  assert.equal(spaced.en![0], "Twenty-three characters", "the 24th character was a space");
});

test("an emoji at the cut is kept or dropped whole", async () => {
  const shown = await check([
    { id: 1, title: "Twenty-three characters😀 more", language: null },
    { id: 2, title: "Twenty-two characters 👨‍👩‍👧‍👦 more", language: null },
  ]);
  assert.deepEqual(shown.en, ["Twenty-three characters😀", "Twenty-two characters 👨‍👩‍👧‍👦"]);
});

test("C1 controls in titles and languages become spaces (inner ones kept, edges trimmed); a title of controls only falls back", async () => {
  const shown = await check([
    { id: 1, title: "A\u0085B\u009f", language: "e\u0080n" },
    { id: 2, title: "\u0085\u0080", language: null },
    { id: 3, title: "", language: "\u009fja\u0085" },
  ]);
  assert.deepEqual(shown.en, ["A B  · e n", "Track 2", "Embedded 3 · ja"]);
  assert.deepEqual(shown["zh-Hans"], ["A B  · e n", "字幕轨 2", "内嵌 3 · ja"]);
});

test("very long CJK titles and languages stay within 80 bytes in every locale, duplicates included", async () => {
  const long = "超长的中文字幕标题".repeat(10);
  const language = "简体中文（中华人民共和国大陆地区）标准普通话";
  const shown = await check([
    { id: 1, title: long, language },
    { id: 2, title: long, language: null },
    { id: 3, title: "", language: language.repeat(3) },
    { id: 3, title: "", language: language.repeat(3) },
    { id: 12345678, title: "", language: "😀".repeat(40) },
    { id: 12345678, title: "", language: "😀".repeat(40) },
    { id: 7, title: "外", language: "zh" },
    { id: 8, title: long.slice(0, 23), language: null },
  ]);
  assert.equal(shown.en![0], long.slice(0, 23));
  assert.equal(shown.en![1], `${long.slice(0, 23)} (2)`);
  assert.equal(shown.ja![1], `${long.slice(0, 23)} (2)`, "a title is data: plain \" (n)\"");
  assert.equal(shown.ja![3], `埋め込み 3 · ${language.slice(0, 10)}（4）`, "a fallback name: the text value \"{}（{}）\"");
  assert.equal(shown.en![2], `Embedded 3 · ${language.slice(0, 10)}`, "32 bytes of the language");
  assert.equal(shown.en![3], `Embedded 3 · ${language.slice(0, 10)} (4)`);
  assert.equal(shown.en![4], `Embedded 12345678 · ${"😀".repeat(8)}`);
  assert.equal(shown.en![5], `Embedded 12345678 · ${"😀".repeat(8)} (6)`);
  assert.equal(shown.en![6], "外 · zh");
  assert.equal(shown.en![7], `${long.slice(0, 23)} (8)`);
});

test("a track titled like another's duplicate key gets the next free number", async () => {
  const shown = await check([
    { id: 1, title: "A", language: null },
    { id: 2, title: "A (3)", language: null },
    { id: 3, title: "A", language: null },
    { id: 4, title: "Off", language: null },
  ]);
  assert.deepEqual(shown.en, ["A", "A (3)", "A (4)", "Off (4)"]);
  assert.deepEqual(shown.ja, ["A", "A (3)", "A (4)", "Off (4)"], "a title is data: plain \" (n)\"");
});

test("an embedded track without a number is \"Track N\", not \"Embedded \" with an edge space", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  for (const language of [null, "ja"])
    host.subtitles.tracks.push({ target: { kind: "embedded", id: undefined as unknown as number }, title: "", language, styleSupported: true, styleReason: null, text: true });
  for (const locale of LOCALES) {
    const { stop } = await startPlugin(activate, { directory, host, locale });
    await host.settle();
    const shown = [1, 2].map((n) => host.command(`track-${n}`).display.path.at(-1) as string);
    for (const s of shown) assert.ok(segmentValid(s), `${locale}: ${JSON.stringify(s)}`);
    if (locale === "en") assert.deepEqual(shown, ["Track 1", "Track 2 · ja"]);
    if (locale === "zh-Hans") assert.deepEqual(shown, ["字幕轨 1", "字幕轨 2 · ja"]);
    await stop();
  }
});

test("a track titled \"Position\" (or \"Off\", \"Cycle\") does not take a fixed item's or submenu's name", async () => {
  const shown = await check([
    { id: 1, title: "Position", language: null },
    { id: 2, title: "Off", language: null },
    { id: 3, title: "Cycle", language: null },
  ]);
  assert.deepEqual(shown.en, ["Position (1)", "Off (2)", "Cycle (3)"]);
});

test("the OSD and the window lists name an id-less embedded track \"Track N\" like the menu", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.subtitles.tracks.push(
    { target: { kind: "embedded", id: 1 }, title: "English", language: "en", styleSupported: true, styleReason: null, text: true },
    { target: { kind: "embedded", id: undefined as unknown as number }, title: "", language: null, styleSupported: true, styleReason: null, text: true },
  );
  const { stop } = await startPlugin(activate, { directory, host });
  await host.settle();
  assert.equal(host.command("track-2").display.path.at(-1), "Track 2");
  await host.command("tune").run();
  const props = host.window("subtitle-tune").props;
  assert.deepEqual(props["track-labels"], ["Subtitles Off", "English · en", "Track 2"]);
  assert.deepEqual(props["secondary-labels"], ["Secondary Off", "English · en", "Track 2"]);
  await host.command("track-2").run();
  await host.settle();
  assert.equal(host.osd.at(-1), "Subtitles: Track 2");
  await stop();
});

test("segmentValid matches the player's rule: 1–80 bytes, no Cc, no edge White_Space", () => {
  for (const ok of ["a", "a b", "a  b", "字".repeat(26), "😀", "a b", "﻿a"]) assert.ok(segmentValid(ok), JSON.stringify(ok));
  for (const bad of ["", " a", "a ", " a", "a　", "\u2028a", "a\u0085", "a\u0080b", "a\u009fb", "a\tb", "a\u007fb", "字".repeat(27)])
    assert.ok(!segmentValid(bad), JSON.stringify(bad));
});

test("without Intl.Segmenter the cut falls back to whole code points", async () => {
  const saved = Object.getOwnPropertyDescriptor(Intl, "Segmenter")!;
  try {
    Reflect.deleteProperty(Intl, "Segmenter"); // a runtime without Intl.Segmenter
    // A fresh copy of the module, evaluated without it.
    const fresh = new URL("../model.ts?no-segmenter", import.meta.url).href;
    const model = (await import(fresh)) as typeof import("../model.ts"); // boundary-reviewed: the package's own model.ts, reloaded
    assert.equal(model.menuCut("abc😀def", 4, 80), "abc😀", "a surrogate pair is one code point");
    assert.equal(model.menuCut("abc😀def", 80, 6), "abc");
    assert.equal(model.menuCut("étude", 1, 80), "e", "code points, not graphemes: the fallback is in use");
    assert.equal(model.menuCut("Director's cut commentary", 15, 80), "Director's cut");
  } finally {
    Object.defineProperty(Intl, "Segmenter", saved);
  }
});
