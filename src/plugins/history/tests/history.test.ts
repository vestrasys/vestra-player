import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { clock, displayName, keyOf, mediaKind, middle } from "../identity.ts";
import activate from "../main.ts";
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
/** Start the plugin; a failed test still stops it (its timers would keep the run alive). */
async function start(host = new TestHost(), dataDirectory?: string) {
  const started = await startPlugin(activate, { directory, host, ...(dataDirectory ? { dataDirectory } : {}) });
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

/** Another plugin (or the bar) opens `source`; it becomes ready, then the duration arrives. */
async function play(host: TestHost, source: string, duration: number | null = 600, position = 0) {
  host.openElsewhere(source);
  await host.settle();
  if (duration !== null) host.setSource({ duration, position });
  await host.settle(WAIT);
}
/** Lines of 打开 ▶ 最近播放 in menu order (one order: declaration order). */
const recentMenu = (host: TestHost) =>
  [...host.commands.values()]
    .filter((c) => c.declaration.path?.[1] === "最近播放")
    .map((c) => c.declaration.path!.at(-1)!);
const records = (host: TestHost) => recentMenu(host).slice(0, -3);
const commandFor = (host: TestHost, title: string) =>
  [...host.commands.entries()].find(([, c]) => c.declaration.path?.at(-1) === title)![0];
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

// ------------------------------------------------------------------ A12

test("A12 menu entries: one order 114 under 打开 ▶ 最近播放, records then the management group", async () => {
  const { host, stop } = await start();
  assert.deepEqual(recentMenu(host), ["（无记录）", "移除无效记录", "清除记录", "记录播放历史"]);
  const empty = host.command("recent.empty");
  assert.equal(empty.state.enabled, false);
  for (const id of ["recent.empty", "recent.prune", "recent.clear", "recent.enabled"])
    assert.equal(host.command(id).declaration.order, 114, id);
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
  await play(host, file("new.mkv"));
  await host.settle(WAIT);
  const saved = JSON.parse(readFileSync(join(data, "recent.json"), "utf8"));
  assert.equal(saved.items.length, 100);
  assert.equal(records(host)[0], "1  new.mkv");
  await stop();
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
    const { declaration } = host.command(id);
    assert.deepEqual(declaration.path, ["播放", "记忆播放位置", title]);
    assert.equal(declaration.order, order);
    assert.equal(declaration.group, group);
  }
  assert.equal(host.command("resume.auto").declaration.radio, true);
  assert.equal(host.command("resume.auto").state.checked, true);
  assert.equal(host.command("resume.ask").state.checked, false);
  assert.equal(host.command("resume.audio").state.checked, false);
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
  assert.equal(window.props.message, "之前有未播放完的。接着播放吗？（05:00）");
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
  const saved = JSON.parse(readFileSync(join(data, "positions.json"), "utf8"));
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
