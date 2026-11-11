import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import type { Locale } from "@vestra-player/sdk";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { bytes, clock, displayName, fileSize, isLocal, keyOf, legacyKeyOf, mediaKind, menuText, middle, middleBytes } from "../identity.ts";
import activate from "../main.ts";
import { POINT_BYTES, segmentValid } from "../menu.ts";
import { timing } from "../store.ts";

Object.assign(timing, { recentWrite: 10, menuRebuild: 10, positionsWrite: 10, positionTick: 25, askTimeout: 400 });
const WAIT = 60; // longer than the shortened merge windows

const directory = new URL("..", import.meta.url);
const scratch = mkdtempSync(join(tmpdir(), "vestra-history-test-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));
let files = 0;
/** A real file (realpath and size work on it). */
function file(name: string, bytes = 10) {
  const folder = join(scratch, String(files++));
  mkdirSync(folder);
  const path = join(folder, name);
  writeFileSync(path, Buffer.alloc(bytes));
  return keyOf(path);
}
const running = new Set<() => Promise<void>>();
/**
 * Start the plugin; a failed test still stops it (its timers would keep the run alive).
 * In Simplified Chinese by default: these tests check what that user sees (the English is checked separately).
 */
async function start(host = new TestHost(), dataDirectory?: string, locale: Locale = "zh-Hans") {
  const started = await startPlugin(activate, { directory, host, locale, ...(dataDirectory ? { dataDirectory } : {}) });
  const stop = async () => {
    running.delete(stop);
    await started.stop();
  };
  running.add(stop);
  return { ...started, stop };
}
test.afterEach(async () => {
  for (const stop of [...running]) await stop();
});

/**
 * A store file once `done` holds for it. The debounced write is the plugin's own
 * work, which `settle` does not wait for (on a loaded machine it ends later):
 * poll, 20 ms at a time, for up to 2 s; the last content read is returned either way.
 */
async function written<T>(host: TestHost, path: string, done: (saved: T) => boolean): Promise<T | undefined> {
  let saved: T | undefined;
  for (let i = 0; i < 100; i++) {
    try {
      saved = JSON.parse(readFileSync(path, "utf8")) as T;
      if (done(saved)) break;
    } catch {
      // not written yet
    }
    await host.settle(20);
  }
  return saved;
}
/** Another plugin (or the bar) opens `source`; it becomes ready, then the duration arrives. */
async function play(host: TestHost, source: string, duration: number | null = 600, position = 0) {
  host.openElsewhere(source);
  await host.settle();
  if (duration !== null) host.setSource({ duration, position });
  await host.settle(WAIT);
}
/** Lines of Open ▸ Recently Played as shown, in menu order (one order: declaration order). */
const recentMenu = (host: TestHost) =>
  [...host.commands.values()]
    .filter((c) => c.declaration.path?.[1] === "Recently Played")
    .map((c) => c.display.path.at(-1)!);
const records = (host: TestHost) => recentMenu(host).slice(0, -3);
const commandFor = (host: TestHost, title: string) =>
  [...host.commands.entries()].find(([, c]) => c.display.path.at(-1) === title)![0];
const userSeek = (host: TestHost, seconds: number) =>
  host.request("source.submit", { operationId: 900000 + seconds, intent: { kind: "control", control: { kind: "seek", seconds } } });
const pluginSeeks = (host: TestHost) =>
  host
    .sent("source.submit")
    .filter((p) => Number(p.operationId) < 900000)
    .map((p) => p.intent as { kind: string; control?: { kind: string; seconds: number } })
    .filter((i) => i.kind === "control" && i.control!.kind === "seek")
    .map((i) => i.control!.seconds);

// ------------------------------------------------------------------ identity

test("identity: real path for files, link without fragment; names, clock, media kinds", () => {
  const path = file("电影.mkv");
  assert.equal(keyOf(path), path);
  assert.equal(keyOf(join(path, "..", ".", "电影.mkv")), path);
  assert.equal(keyOf("/no/such/a.mkv"), "/no/such/a.mkv");
  assert.equal(keyOf("https://h.example/v/a.mp4?x=1#t=30"), "https://h.example/v/a.mp4?x=1");
  assert.equal(displayName("/Users/x/Movies/b.mkv"), "b.mkv");
  assert.equal(displayName("https://user:secret@h.example/videos/%E7%AC%AC1%E9%9B%86.mp4"), "h.example › 第1集.mp4");
  assert.equal(displayName("https://h.example/"), "h.example");
  assert.equal([...displayName(`https://h.example/${"x".repeat(100)}`)].length, 60);
  assert.equal(middle("abcdefghij", 5), "ab…ij");
  assert.equal(clock(754), "12:34");
  assert.equal(clock(3723), "1:02:03");
  assert.equal(mediaKind("/a/B.MKV"), "video");
  assert.equal(mediaKind("/a/b.flac"), "audio");
  assert.equal(mediaKind("https://h/a.mp3?x"), "audio");
  assert.equal(mediaKind("https://h/watch?v=1"), null);
});

const windows = process.platform === "win32";

test("identity on every OS: a file: URL is its file, stream links stay links", () => {
  const path = file("片 #1.mkv", 7);
  const url = pathToFileURL(path).href;
  assert.ok(isLocal(url));
  assert.equal(keyOf(url), path);
  assert.equal(keyOf(`${url}#t=5`), path);
  assert.equal(displayName(url), "片 #1.mkv");
  assert.equal(fileSize(url), 7);
  assert.equal(mediaKind(url), "video");
  assert.equal(legacyKeyOf(url), url, "0.1.3 keyed a file: URL as link text");
  // 0.1.3 cut a Windows path at its `#` like a link.
  assert.equal(legacyKeyOf(path), windows ? path.slice(0, path.indexOf("#")) : null);
  for (const link of ["https://h.example/v/a.mp4#t=1", "rtsp://cam.local/live/main", "rtmp://h.example/app/key"]) {
    assert.ok(!isLocal(link), link);
    assert.equal(fileSize(link), null);
    assert.equal(legacyKeyOf(link), null, link);
  }
  assert.equal(displayName("rtsp://cam.local/live/main"), "cam.local › main");
  assert.equal(keyOf("rtsp://cam.local/live/main#x"), "rtsp://cam.local/live/main");
});

test("identity of Windows paths: files on Windows, links shown by their file name on macOS", () => {
  const sources = ["C:\\Users\\x\\Movies\\b.mkv", "C:/Users/x/Movies/b.mkv", "\\\\server\\share\\Movies\\b.mkv"];
  for (const source of sources) {
    assert.equal(isLocal(source), windows, source);
    assert.equal(displayName(source), "b.mkv", source);
    assert.equal(mediaKind(source), "video", source);
  }
  // A missing file keeps its path as the key (the fragment is part of a file name).
  assert.equal(keyOf("C:\\no\\such\\a#1.mkv"), windows ? "C:\\no\\such\\a#1.mkv" : "C:\\no\\such\\a");
  assert.equal(legacyKeyOf("C:\\no\\such\\a#1.mkv"), windows ? "C:\\no\\such\\a" : null);
  // A share as a file: URL: a UNC path on Windows; macOS has no path for it.
  assert.equal(isLocal("file://server/share/b.mkv"), windows);
  assert.equal(displayName("file://server/share/b.mkv"), windows ? "b.mkv" : "server › b.mkv");
  if (windows) {
    // One file however its path is spelled.
    const path = file("c.mkv");
    assert.equal(keyOf(path.replaceAll("\\", "/")), path);
    assert.equal(keyOf(join(path, "..", ".", "c.mkv").replaceAll("\\", "/")), path);
  }
});

// ------------------------------------------------------------------ A12

test("English by default: menus, messages and the ask window in the source text", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, plugin, stop } = await start(undefined, undefined, "en");
  assert.equal(plugin.i18n.locale, "en");
  const shown = (id: string) => host.command(id).display.path;
  assert.deepEqual(host.command("recent.prune").declaration.path, ["@open", "Recently Played", "Remove Invalid Entries"]);
  assert.deepEqual(
    ["recent.empty", "recent.prune", "recent.clear", "recent.enabled"].map(shown),
    [
      ["Open", "Recently Played", "(No Entries)"],
      ["Open", "Recently Played", "Remove Invalid Entries"],
      ["Open", "Recently Played", "Clear Entries"],
      ["Open", "Recently Played", "Record Playback History"],
    ],
  );
  // Menu sweep (B5): Resume Playback is in the command palette only ("Resume Playback: Title").
  assert.deepEqual(host.command("resume.off").declaration.path, []);
  assert.deepEqual(
    ["resume.off", "resume.auto", "resume.ask", "resume.audio", "resume.clear"].map((id) => [shown(id), host.command(id).display.title]),
    [
      [[], "Don't Resume"],
      [[], "Resume Automatically"],
      [[], "Ask Every Time"],
      [[], "Also Remember Audio Files"],
      [[], "Clear All Saved Positions"],
    ],
  );
  assert.equal(host.command("resume.off").display.category, "Resume Playback");

  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  assert.deepEqual(records(host), ["1  b.mkv", "2  a.mkv (05:00)"]);
  await play(host, a);
  assert.equal(host.osd.at(-1), "Resuming from 05:00");
  await host.command("resume.clear").run();
  assert.equal(host.osd.at(-1), "All saved positions cleared");
  await host.command("recent.prune").run();
  assert.equal(host.osd.at(-1), "Removed 0 invalid entries");
  await host.command("recent.clear").run();
  assert.equal(host.osd.at(-1), "Playback history cleared");
  assert.equal(
    host.translate("You didn't finish this last time. Continue? ({})", ["05:00"]),
    "You didn't finish this last time. Continue? (05:00)",
  );
  await stop();
});

test("A12 menu entries: one order 119 under 打开 ▸ 最近播放 (last in Open ▸), records then the management group", async () => {
  const { host, stop } = await start();
  assert.deepEqual(recentMenu(host), ["（无记录）", "移除无效记录", "清除记录", "记录播放历史"]);
  const empty = host.command("recent.empty");
  assert.equal(empty.state.enabled, false);
  for (const id of ["recent.empty", "recent.prune", "recent.clear", "recent.enabled"])
    assert.equal(host.command(id).declaration.order, 119, id);
  assert.equal(empty.declaration.group, "recent");
  assert.equal(host.command("recent.prune").declaration.group, "recent-manage");
  assert.equal(host.command("recent.prune").state.enabled, false);
  assert.equal(host.command("recent.enabled").state.checked, true);
  await stop();
});

test("A12 opening a, b, then a again lists a, b; any opener is recorded; errors and streams are not", async () => {
  const [a, b, c] = [file("a.mkv"), file("b.mkv"), file("c.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  await play(host, b);
  await play(host, a);
  assert.deepEqual(records(host), ["1  a.mkv", "2  b.mkv"]);
  host.openElsewhere(c, "error");
  host.openElsewhere("/x/stream", "loading");
  host.setSource({ kind: "producer", phase: "ready" });
  await host.settle(WAIT);
  assert.deepEqual(records(host), ["1  a.mkv", "2  b.mkv"]);
  assert.equal(host.command("recent.item.0").declaration.group, "recent");
  await stop();
});

test("A12 a replay of the same source is not recorded again", async () => {
  const a = file("a.mkv");
  const { host, stop } = await start();
  await play(host, a);
  await host.command("recent.clear").run();
  assert.equal(host.osd.at(-1), "已清除播放记录");
  host.setSource({ phase: "ended", terminalReason: "natural" });
  host.setSource({ phase: "ready", position: 0, terminalReason: null });
  await host.settle(WAIT);
  assert.deepEqual(recentMenu(host)[0], "（无记录）");
  await stop();
});

test("A12 clicking a record opens it unconditionally; a deleted file is reported, kept, and pruned", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  await play(host, b);
  await play(host, "https://h.example/v/c.mp4#t=1");
  assert.deepEqual(records(host), ["1  h.example › c.mp4", "2  b.mkv", "3  a.mkv"]);
  await host.command(commandFor(host, "3  a.mkv")).run();
  await host.settle();
  assert.equal(host.source.source, a);
  assert.equal(host.source.phase, "loading");

  unlinkSync(b);
  const opened = host.sent("source.submit").length;
  await host.command(commandFor(host, "2  b.mkv")).run();
  await host.settle();
  assert.equal(host.osd.at(-1), "文件不存在：b.mkv");
  assert.equal(host.sent("source.submit").length, opened);
  host.setSource({ phase: "ready", duration: 600, position: 0 });
  await host.settle(WAIT);
  assert.deepEqual(records(host), ["1  a.mkv", "2  h.example › c.mp4", "3  b.mkv"]);

  await host.command("recent.prune").run();
  await host.settle(WAIT);
  assert.equal(host.osd.at(-1), "已移除 1 条无效记录");
  assert.deepEqual(records(host), ["1  a.mkv", "2  h.example › c.mp4"]);
  await stop();
});

test("A12 a load failure after the click is shown", async () => {
  const a = file("a.mkv");
  const { host, stop } = await start();
  await play(host, a);
  await play(host, file("b.mkv"));
  await host.command(commandFor(host, "2  a.mkv")).run();
  await host.settle();
  host.setSource({ phase: "error", error: "不支持的格式" } as never);
  await host.settle();
  assert.match(host.osd.at(-1)!, /^无法打开：/);
  await stop();
});

test("A12 记录播放历史 off: nothing new is recorded, existing records stay; the choice survives a restart", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const [a, d] = [file("a.mkv"), file("d.mkv")];
  let { host, stop } = await start(undefined, data);
  await play(host, a);
  await host.command("recent.enabled").run();
  await host.settle();
  assert.equal(host.command("recent.enabled").state.checked, false);
  await play(host, d);
  assert.deepEqual(records(host), ["1  a.mkv"]);
  await stop();

  ({ host, stop } = await start(undefined, data));
  assert.deepEqual(records(host), ["1  a.mkv"]);
  assert.equal(host.command("recent.enabled").state.checked, false);
  const saved = JSON.parse(readFileSync(join(data, "recent.json"), "utf8"));
  assert.equal(saved.version, 1);
  assert.equal(saved.enabled, false);
  assert.deepEqual(
    saved.items.map((i: { key: string; kind: string }) => [i.key, i.kind]),
    [[a, "file"]],
  );
  await stop();
});

test("A12 keeps 100 records, shows 20 numbered 1–9, 0, then none; a same-named later record stays distinct", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const items = Array.from({ length: 105 }, (_, i) => ({
    key: `/m/${i}/v${i % 15 === 14 ? "same" : i}.mkv`,
    source: `/m/${i}/v${i % 15 === 14 ? "same" : i}.mkv`,
    kind: "file",
    openedAt: new Date(0).toISOString(),
  }));
  writeFileSync(join(data, "recent.json"), JSON.stringify({ version: 1, enabled: true, items }));
  const { host, stop } = await start(undefined, data);
  const lines = records(host);
  assert.equal(lines.length, 20);
  assert.equal(lines[0], "1  v0.mkv");
  assert.equal(lines[8], "9  v8.mkv");
  assert.equal(lines[9], "0  v9.mkv");
  assert.equal(lines[10], "v10.mkv");
  assert.equal(lines[14], "vsame.mkv");
  assert.equal(lines[19], "v19.mkv");
  const fresh = file("new.mkv");
  await play(host, fresh);
  await host.settle(WAIT);
  type Saved = { items: { key: string }[] };
  const saved = await written<Saved>(host, join(data, "recent.json"), (s) => s.items[0]?.key === fresh);
  assert.ok(saved, "recent.json written");
  assert.equal(saved.items[0]?.key, fresh);
  assert.equal(saved.items.length, 100);
  assert.equal(records(host)[0], "1  new.mkv");
  await stop();
});

/** A record line as the player checks and keys it: the English rendering of `{} ({})`. */
const englishLine = (segment: unknown) => {
  if (typeof segment === "string") return segment;
  const { $vestraText, args = [] } = segment as { $vestraText: string; args?: unknown[] };
  let next = 0;
  return $vestraText.replace(/\{\}/g, () => String(args[next++]));
};
/** Every record line passes the player's check of a path segment (and its title, 128 bytes), as declared and as shown. */
function assertPlayerAccepts(host: TestHost) {
  const lines = [...host.commands.entries()].filter(([id]) => id.startsWith("recent.item."));
  const keys = new Set<string>();
  for (const [id, { declaration, display }] of lines) {
    const key = englishLine(declaration.path!.at(-1));
    const shown = display.path.at(-1)!;
    for (const text of [key, shown]) {
      assert.ok(segmentValid(text), `${id}: ${JSON.stringify(text)} (${bytes(text)} bytes)`);
      assert.equal(Buffer.from(text).toString(), text, `${id}: no broken character in ${JSON.stringify(text)}`);
    }
    keys.add(key);
  }
  assert.equal(keys.size, lines.length, "every line is its own menu path");
  return lines.map(([, { display }]) => display.path.at(-1)!);
}

test("A12 lines fit the player's 80 bytes: long CJK names with a point in Chinese, emoji, edge spaces, controls, duplicates past the tenth", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const cjk = "超长的中文文件名".repeat(12) + ".mkv"; // 100 characters, 292 bytes
  const family = "👨‍👩‍👧‍👦".repeat(10) + ".mkv"; // ZWJ sequences of 25 bytes
  const names = [
    cjk,
    "  spaced name.mkv  ",
    family,
    "tab\there\u0007.mkv",
    " \t ",
    "a.mkv",
    "b.mkv",
    "c.mkv",
    "d.mkv",
    "e.mkv",
    ...Array.from({ length: 10 }, () => cjk), // 11th to 20th: the same name, shortened alike
  ];
  const items = names.map((name, i) => ({ key: `/m/${i}/${name}`, source: `/m/${i}/${name}`, kind: "file", openedAt: new Date(0).toISOString() }));
  // Points (one past an hour), shown in Chinese brackets.
  const memo = (position: number) => ({ position, duration: 7200, size: null, updatedAt: 1 });
  const points = Object.fromEntries(items.filter((_, i) => [0, 1, 2].includes(i)).map((item, i) => [item.key, memo(i ? 300 : 3723)]));
  writeFileSync(join(data, "recent.json"), JSON.stringify({ version: 1, enabled: true, items }));
  writeFileSync(join(data, "positions.json"), JSON.stringify({ version: 1, mode: "auto", audio: false, items: points }));
  const { host, stop } = await start(undefined, data);
  const lines = assertPlayerAccepts(host);
  assert.equal(lines.length, 20, "no record left out");
  assert.match(lines[0]!, /^1 {2}超长的中文.*….*\.mkv（1:02:03）$/u);
  assert.equal(lines[1], "2  spaced name.mkv（05:00）", "edge whitespace trimmed");
  assert.match(lines[2]!, /^3 {2}(👨‍👩‍👧‍👦)+…(👨‍👩‍👧‍👦)*\.mkv（05:00）$/u, "whole emoji sequences");
  assert.equal(lines[3], "4  tab here .mkv", "controls become spaces");
  assert.ok(lines[4]!.startsWith("5  /m/4"), "a name of whitespace only shows its path");
  for (const [i, line] of lines.slice(10).entries())
    assert.match(line, i ? new RegExp(`^超长.*….*\\.mkv \\(${i + 1}\\)$`, "u") : /^超长.*….*\.mkv$/u, line);
  await stop();

  // In English, with a leading space past the tenth (no number in front).
  const english = mkdtempSync(join(scratch, "data-"));
  const plain = Array.from({ length: 10 }, (_, i) => `p${i}.mkv`);
  const items2 = [...plain, "   leading.mkv", cjk].map((name, i) => ({ key: `/e/${i}/${name}`, source: `/e/${i}/${name}`, kind: "file", openedAt: new Date(0).toISOString() }));
  writeFileSync(join(english, "recent.json"), JSON.stringify({ version: 1, enabled: true, items: items2 }));
  writeFileSync(join(english, "positions.json"), JSON.stringify({ version: 1, mode: "auto", audio: false, items: { [items2[11]!.key]: memo(3723) } }));
  const en = await start(undefined, english, "en");
  const shown = assertPlayerAccepts(en.host);
  assert.equal(shown[10], "leading.mkv");
  assert.match(shown[11]!, /^超长.*….*\.mkv \(1:02:03\)$/u);
  await en.stop();
});

test("A12 no line repeats another as the player keys them: a name like a line with its point, the fixed items", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const plain = Array.from({ length: 10 }, (_, i) => `p${i}.mkv`);
  const names = [...plain, "X (1:02:03)", "X", "Clear Entries", "Remove Invalid Entries", "Record Playback History", "(No Entries)"];
  const items = names.map((name, i) => ({ key: `/k/${i}/${name}`, source: `/k/${i}/${name}`, kind: "file", openedAt: new Date(0).toISOString() }));
  const points = { [items[11]!.key]: { position: 3723, duration: 7200, size: null, updatedAt: 1 } };
  writeFileSync(join(data, "recent.json"), JSON.stringify({ version: 1, enabled: true, items }));
  writeFileSync(join(data, "positions.json"), JSON.stringify({ version: 1, mode: "auto", audio: false, items: points }));
  const { host, stop } = await start(undefined, data, "en");
  const shown = assertPlayerAccepts(host);
  assert.deepEqual(shown.slice(10), [
    "X (1:02:03)",
    "X (2) (1:02:03)",
    "Clear Entries (2)",
    "Remove Invalid Entries (2)",
    "Record Playback History (2)",
    "(No Entries) (2)",
  ]);
  const keys = [...host.commands.values()]
    .filter((c) => c.declaration.path?.[1] === "Recently Played")
    .map((c) => englishLine(c.declaration.path!.at(-1)));
  assert.equal(keys.length, 16 + 3);
  assert.equal(new Set(keys).size, keys.length, "no two lines of the submenu share a key");
  await stop();
});

test("identity: middleBytes shortens by UTF-8 bytes in the middle, whole characters only; segmentValid is the player's rule", () => {
  assert.equal(middleBytes("abcdefghij", 8), "abc…ij");
  assert.equal(middleBytes("abc", 3), "abc");
  for (const [text, max] of [
    ["电影电影电影电影电影", 13],
    ["😀😀😀😀😀😀", 15],
    ["e\u0301e\u0301e\u0301e\u0301e\u0301", 10],
  ] as const) {
    const short = middleBytes(text, max);
    assert.ok(bytes(short) <= max, short);
    assert.equal(Buffer.from(short).toString(), short);
    assert.ok(short.includes("…"));
  }
  assert.equal(middleBytes("电影电影电影电影电影", 13), "电…电影"); // 3 + 3 + 6: no half character
  assert.equal(middleBytes("e\u0301e\u0301e\u0301e\u0301", 8), "e\u0301…", "an accent stays with its letter");
  assert.equal(menuText(" \u0000a\tb\u0085 "), "a b");
  assert.ok(segmentValid("x".repeat(80)));
  assert.ok(!segmentValid("x".repeat(81)));
  assert.ok(!segmentValid("电".repeat(27)), "81 bytes");
  assert.ok(!segmentValid(" a") && !segmentValid("a\u3000") && !segmentValid("a\nb") && !segmentValid(""));
});

test("locales: no translation of {} ({}) adds more than POINT_BYTES around the name and the point", () => {
  const checked: string[] = [];
  for (const file of readdirSync(new URL("../locales/", import.meta.url)).filter((name) => name.endsWith(".po"))) {
    const po = readFileSync(new URL(`../locales/${file}`, import.meta.url), "utf8");
    const translated = /msgid "\{\} \(\{\}\)"\r?\nmsgstr "(.*)"/.exec(po)?.[1];
    assert.ok(translated, `${file}: {} ({}) is translated`);
    assert.ok(bytes(translated.replaceAll("{}", "")) <= POINT_BYTES, `${file}: ${translated}`);
    checked.push(file);
  }
  for (const file of ["ja.po", "zh-Hans.po"]) assert.ok(checked.includes(file), file);
});

test("A12 menu rebuilds within the merge window are one rebuild", async () => {
  const { host, stop } = await start();
  const registrations = () => host.sent("commands.register").filter((p) => p.id === "recent.prune").length;
  const before = registrations();
  host.openElsewhere(file("a.mkv"));
  host.openElsewhere(file("b.mkv"));
  host.openElsewhere(file("c.mkv"));
  await host.settle(WAIT);
  assert.equal(registrations(), before + 1);
  assert.deepEqual(records(host), ["1  c.mkv", "2  b.mkv", "3  a.mkv"]);
  await stop();
});

// ------------------------------------------------------------------ B25

test("B25 menu: 不记忆 / 自动继续 / 每次询问 radios 250–252, audio 253 and clear 254", async () => {
  const { host, stop } = await start();
  const expected: [string, string, number, string][] = [
    ["resume.off", "不记忆", 250, "domain"],
    ["resume.auto", "自动继续", 251, "domain"],
    ["resume.ask", "每次询问", 252, "domain"],
    ["resume.audio", "同时记忆音频文件", 253, "resume-options"],
    ["resume.clear", "清除所有记忆点", 254, "resume-options"],
  ];
  for (const [id, title, order, group] of expected) {
    const { declaration, display } = host.command(id);
    // Menu sweep (B5): palette only, "记忆播放位置: <title>".
    assert.deepEqual([display.path, display.title, display.category], [[], title, "记忆播放位置"]);
    assert.equal(declaration.order, order);
    assert.equal(declaration.group, group);
  }
  assert.equal(host.command("resume.auto").declaration.radio, true);
  assert.equal(host.command("resume.auto").state.checked, true);
  assert.equal(host.command("resume.ask").state.checked, false);
  assert.equal(host.command("resume.audio").state.checked, false);
  await stop();
});

// ------------------------------------------------------------------ palette

test("palette: every command has a category (the submenu's label), empty and with records, in English and Chinese", async () => {
  const a = file("a.mkv");
  const expected = {
    en: { recent: "Recently Played", resume: "Resume Playback", reason: "No recently played items" },
    "zh-Hans": { recent: "最近播放", resume: "记忆播放位置", reason: "没有最近播放的项目" },
  } as const;
  for (const locale of ["en", "zh-Hans"] as const) {
    const { host, stop } = await start(undefined, undefined, locale);
    const { recent, resume, reason } = expected[locale];
    const check = () => {
      assert.ok(host.commands.size > 0);
      for (const [id, { declaration, display }] of host.commands) {
        assert.ok(typeof display.category === "string" && display.category.trim() !== "", `${locale}: ${id} has no category`);
        // Same (former) submenu, same category: the menu label as shown; Recently Played is still a submenu.
        if (id.startsWith("recent.")) assert.equal(display.category, display.path[1], `${locale}: ${id}`);
        else assert.deepEqual(display.path, [], `${locale}: ${id} is palette only`);
        assert.equal(display.category, id.startsWith("recent.") ? recent : resume, `${locale}: ${id}`);
        assert.equal(declaration.home, undefined, `${locale}: ${id} has no window of its own`);
      }
    };
    // Empty: the management items are disabled with a reason, so the palette still lists them.
    check();
    for (const id of ["recent.prune", "recent.clear"]) {
      assert.equal(host.command(id).state.enabled, false, id);
      assert.equal(host.command(id).display.reason, reason, `${locale}: ${id}`);
    }
    await play(host, a);
    assert.ok(host.commands.has("recent.item.0"));
    check();
    for (const id of ["recent.prune", "recent.clear"]) {
      assert.equal(host.command(id).state.enabled, true, id);
      assert.equal(host.command(id).display.reason ?? null, null, `${locale}: ${id}`);
    }
    await stop();
  }
});

test("destructive: the three commands that remove records or saved positions say so; no other command does", async () => {
  const destructive = ["recent.clear", "recent.prune", "resume.clear"];
  const { host, stop } = await start();
  const check = () => {
    for (const [id, { declaration }] of host.commands)
      assert.equal(declaration.destructive, destructive.includes(id) ? true : undefined, id);
  };
  check(); // empty
  await play(host, file("a.mkv"));
  assert.ok(host.commands.has("recent.item.0"));
  check(); // with records: the rebuilt submenu keeps the marks
  for (const id of destructive) assert.ok(host.commands.has(id), id);
  await stop();
});

test("B25 left at 05:00 and opened again: continues from 04:58 with 「从 05:00 继续播放」; A12 shows the point", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  assert.deepEqual(records(host), ["1  b.mkv", "2  a.mkv（05:00）"]);
  assert.deepEqual(pluginSeeks(host), []);
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), [298]);
  assert.equal(host.source.position, 298);
  assert.equal(host.osd.at(-1), "从 05:00 继续播放");
  await stop();
});

test("B25 near the end, short sources, the start, and a natural end are not remembered", async () => {
  const [a, short, b] = [file("a.mkv"), file("short.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  await play(host, a);
  host.setSource({ position: 590 }); // 10 s before the end
  await play(host, b);
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), [298]); // the second opening starts from the beginning

  await play(host, short, 30);
  host.setSource({ position: 20 });
  await play(host, b);
  await play(host, short, 30);
  assert.deepEqual(pluginSeeks(host), [298]);

  host.setSource({ position: 200 });
  host.setSource({ phase: "ended", terminalReason: "natural", position: 200 });
  await play(host, b);
  await play(host, short, 30);
  assert.deepEqual(pluginSeeks(host), [298]);

  await play(host, a);
  host.setSource({ position: 300 });
  host.setSource({ position: 5 }); // back to the start
  await play(host, b);
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), [298]);
  await stop();
});

test("B25 a natural end forgets the point", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300, paused: true });
  await host.settle(WAIT);
  assert.equal(records(host)[0], "1  a.mkv（05:00）");
  host.setSource({ phase: "ended", terminalReason: "natural", position: 300 });
  await host.settle(WAIT);
  assert.equal(records(host)[0], "1  a.mkv");
  await play(host, b);
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), []);
  await stop();
});

test("B25 audio files only with 同时记忆音频文件; unknown extensions ask player.stats()", async () => {
  const [song, b] = [file("song.mp3"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, song);
  host.setSource({ position: 100 });
  await play(host, b);
  await play(host, song);
  assert.deepEqual(pluginSeeks(host), []);

  await host.command("resume.audio").run();
  assert.equal(host.command("resume.audio").state.checked, true);
  host.setSource({ position: 100 });
  await play(host, b);
  await play(host, song);
  assert.deepEqual(pluginSeeks(host), [98]);

  // A link without a media extension: the backend says whether it has video.
  await host.command("resume.audio").run();
  host.stats = { ...host.stats, video: { codec: "h264", width: 1, height: 1, fps: 25 } } as never;
  await play(host, "https://h.example/watch?v=1");
  host.setSource({ position: 100 });
  await play(host, b);
  await play(host, "https://h.example/watch?v=1#t=5");
  assert.deepEqual(pluginSeeks(host), [98, 98]);
  await stop();
});

test("B25 someone else's seek after ready wins (stale): no resume, no message", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  host.openElsewhere(a);
  await host.settle();
  await userSeek(host, 100);
  host.setSource({ duration: 600 });
  await host.settle(WAIT);
  assert.deepEqual(pluginSeeks(host), []);
  assert.equal(host.source.position, 100);
  assert.ok(!host.osd.some((t) => t.startsWith("从")));
  await stop();
});

test("B25 每次询问: plays from the start and asks; 接着播放 seeks, a timeout or a new source starts over", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await host.command("resume.ask").run();
  assert.equal(host.command("resume.ask").state.checked, true);
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  await play(host, a);
  const window = host.window("resume");
  assert.equal(window.file, "ui/resume.slint");
  assert.equal(window.visible, true);
  assert.equal(window.props.position, "05:00");
  assert.equal(host.translate("You didn't finish this last time. Continue? ({})", ["05:00"]), "之前有未播放完的。接着播放吗？（05:00）");
  assert.deepEqual(pluginSeeks(host), []);
  // Still playing from the start while asking: the point is not overwritten.
  host.setSource({ position: 3 });
  await host.settle(WAIT);
  window.emit("resume");
  await host.settle();
  assert.equal(window.visible, false);
  assert.deepEqual(pluginSeeks(host), [298]);
  assert.equal(host.osd.at(-1), "从 05:00 继续播放");

  host.setSource({ position: 300 });
  await play(host, b);
  await play(host, a);
  assert.equal(window.visible, true);
  await host.settle(timing.askTimeout + WAIT);
  assert.equal(window.visible, false);
  assert.deepEqual(pluginSeeks(host), [298]);

  host.setSource({ position: 300 });
  await play(host, b);
  await play(host, a);
  assert.equal(window.visible, true);
  await play(host, b);
  assert.equal(window.visible, false);
  window.emit("resume"); // a late answer does nothing
  await host.settle();
  assert.deepEqual(pluginSeeks(host), [298]);
  await stop();
});

test("B25 每次询问: 从头播放 closes the window and playing on records the new position", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await host.command("resume.ask").run();
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  await play(host, a);
  host.window("resume").emit("restart");
  await host.settle();
  assert.equal(host.window("resume").visible, false);
  host.setSource({ position: 120 });
  await play(host, b);
  assert.equal(records(host)[1], "2  a.mkv（02:00）");
  await stop();
});

test("B25 a replaced file (other size) drops its point; 不记忆 neither reads nor writes", async () => {
  const [a, b] = [file("a.mkv", 10), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300 });
  await play(host, b);
  writeFileSync(a, Buffer.alloc(20));
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), []);
  assert.equal(records(host)[0], "1  a.mkv");

  host.setSource({ position: 300 });
  await play(host, b);
  await host.command("resume.off").run();
  await play(host, a);
  host.setSource({ position: 400 });
  await play(host, b);
  assert.deepEqual(pluginSeeks(host), []);
  assert.equal(records(host)[1], "2  a.mkv（05:00）");
  await host.command("resume.auto").run();
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), [298]);
  await stop();
});

test("B25 清除所有记忆点 clears them; a replay of the same source is not resumed", async () => {
  const [a, b] = [file("a.mkv"), file("b.mkv")];
  const { host, stop } = await start();
  await play(host, a);
  host.setSource({ position: 300, paused: true });
  await host.settle(WAIT);
  host.setSource({ phase: "ended", terminalReason: "stopped" });
  host.setSource({ phase: "ready", position: 0, paused: false, terminalReason: null });
  await host.settle(WAIT);
  assert.deepEqual(pluginSeeks(host), []);
  await host.command("resume.clear").run();
  assert.equal(host.osd.at(-1), "已清除所有记忆点");
  await play(host, b);
  await play(host, a);
  assert.deepEqual(pluginSeeks(host), []);
  await stop();
});

test("B25 the playing position is saved every tick, so a forced quit loses little; a reload does not resume", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const a = file("a.mkv");
  const { host, stop } = await start(undefined, data);
  await play(host, a);
  host.setSource({ position: 300 });
  await host.settle(timing.positionTick * 2 + WAIT);
  // Read while still playing (before stop() writes it anyway).
  type Saved = { version: number; mode: string; audio: boolean; items: Record<string, { position: number; duration: number; size: number }> };
  const saved = await written<Saved>(host, join(data, "positions.json"), (s) => s.items?.[a] !== undefined);
  assert.ok(saved, "positions.json written while playing");
  assert.equal(saved.version, 1);
  assert.equal(saved.mode, "auto");
  assert.equal(saved.audio, false);
  assert.deepEqual(Object.keys(saved.items), [a]);
  assert.equal(saved.items[a].position, 300);
  assert.equal(saved.items[a].duration, 600);
  assert.equal(saved.items[a].size, 10);
  await stop();

  // Restarted while the same source plays: tracked, not resumed.
  const host2 = new TestHost();
  host2.openElsewhere(a);
  host2.setSource({ duration: 600, position: 310 });
  const third = await start(host2, data);
  await host2.settle(WAIT);
  assert.deepEqual(pluginSeeks(host2), []);
  await third.stop();
});

test("B25 stopping the plugin remembers the last position", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const a = file("a.mkv");
  const { host, stop } = await start(undefined, data);
  await play(host, a);
  host.setSource({ position: 123 });
  await host.settle();
  await stop();
  const saved = JSON.parse(readFileSync(join(data, "positions.json"), "utf8"));
  assert.equal(saved.items[a].position, 123);

  const next = await start(undefined, data);
  await play(next.host, a);
  assert.deepEqual(pluginSeeks(next.host), [121]);
  assert.equal(next.host.osd.at(-1), "从 02:03 继续播放");
  await next.stop();
});

test("stopping: a menu change during the teardown rebuilds nothing (no 'already declared')", async () => {
  const { host, stop } = await start();
  await play(host, file("a.mkv"));
  host.setSource({ position: 300, paused: true });
  await host.settle(WAIT);
  assert.equal(records(host)[0], "1  a.mkv（05:00）");
  // The points are cleared (the menu lines change) just before stop(); the
  // merge window ends while stop() is removing the submenu.
  let removing = false;
  host.handle("commands.unregister", async (p) => {
    if (!removing && String(p.id).startsWith("recent.")) {
      removing = true;
      await new Promise((resolve) => setTimeout(resolve, timing.menuRebuild + WAIT));
    }
    host.commands.delete(String(p.id));
    return null;
  });
  const cleared = host.command("resume.clear").run();
  await stop();
  await cleared;
  assert.ok(removing, "the submenu was removed");
  assert.deepEqual(recentMenu(host), []);
});

test("storage: records saved under the 0.1.3 keys (file: URLs; Windows paths) are read, resumed and moved to the new key", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  const url = pathToFileURL(file("u.mkv")).href;
  // On Windows 0.1.3 kept a drive path as link text: kind "url", the key as spelled.
  const drive = windows ? file("w.mkv").replaceAll("\\", "/") : null;
  const missing = windows ? join(scratch, "gone", "m.mkv") : null;
  const legacy = [url, drive, missing].filter((source) => source !== null);
  const openedAt = new Date(0).toISOString();
  const items = legacy.map((source) => ({ key: source, source, kind: "url", openedAt }));
  // 0.1.3 saw no file there, so it saved no size.
  const memo = { position: 300, duration: 600, size: null, updatedAt: 1 };
  writeFileSync(join(data, "recent.json"), JSON.stringify({ version: 1, enabled: true, items }));
  writeFileSync(
    join(data, "positions.json"),
    JSON.stringify({ version: 1, mode: "auto", audio: false, items: Object.fromEntries(legacy.map((key) => [key, memo])) }),
  );
  const { host, stop } = await start(undefined, data);
  const names = legacy.map((source) => basename(source));
  assert.deepEqual(
    records(host),
    names.map((name, index) => `${index + 1}  ${name}（05:00）`),
  );
  for (const source of [url, drive].filter((source) => source !== null)) {
    await play(host, source);
    assert.equal(host.osd.at(-1), "从 05:00 继续播放", source);
  }
  assert.deepEqual(pluginSeeks(host), windows ? [298, 298] : [298]);
  assert.equal(records(host).length, legacy.length, "no record twice");
  await play(host, file("x.mkv"));
  await stop();
  const recent = JSON.parse(readFileSync(join(data, "recent.json"), "utf8"));
  const positions = JSON.parse(readFileSync(join(data, "positions.json"), "utf8"));
  const moved = [url, drive].filter((source) => source !== null).map((source) => keyOf(source));
  for (const key of moved) {
    assert.ok(recent.items.some((item: { key: string; kind: string }) => item.key === key && item.kind === "file"), key);
    assert.equal(positions.items[key]?.size, 10, key);
  }
  for (const source of [url, drive]) if (source) assert.ok(!(source in positions.items), source);

  if (missing) {
    // A missing Windows file saved as "url" is now a file: Remove Invalid Entries removes it.
    const next = await start(undefined, data);
    await next.host.command("recent.prune").run();
    await next.host.settle(WAIT);
    assert.equal(next.host.osd.at(-1), "已移除 1 条无效记录");
    await next.stop();
  }
});

test("storage: a damaged file is kept as .bad-<time> and the defaults apply; one bad field keeps the rest", async () => {
  const data = mkdtempSync(join(scratch, "data-"));
  writeFileSync(join(data, "recent.json"), "{ not json");
  writeFileSync(join(data, "positions.json"), JSON.stringify({ version: 1, mode: "sometimes", audio: true, items: { "/x.mkv": { position: "bad" } } }));
  const { host, stop } = await start(undefined, data);
  assert.deepEqual(records(host), ["（无记录）"]);
  assert.equal(host.command("recent.enabled").state.checked, true);
  assert.equal(host.command("resume.auto").state.checked, true, "an unknown mode falls back to 自动继续");
  assert.equal(host.command("resume.audio").state.checked, true, "the valid field is kept");
  await stop();
  const names = readdirSync(data);
  assert.ok(names.some((name) => /^recent\.json\.bad-\d+$/.test(name)), names.join());
  assert.ok(!names.some((name) => name.startsWith("positions.json.bad")), "an object with bad fields is not discarded");
});
