import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readdir, readFile, realpath, rm, symlink, utimes, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "../../../sdk/plugin/index.ts";
import { HostError, startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

type ViewRow = { rowId: string; label: string; duration: string; tooltip: string; selected: boolean; current: boolean };
type Intent = { kind: string; source?: string; expected?: Json };
const pause = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

/** ffprobe (through `plugin.tools`) answers from a table of seconds by file name; unknown names fail. */
const probeTable = (seconds: Record<string, number> = {}) => (args: string[]) => {
  const name = basename(args.at(-1)!);
  if (!(name in seconds)) throw new HostError("failed", "no duration");
  return { stdout: JSON.stringify({ format: { duration: String(seconds[name]) } }) };
};

async function start(
  options: { data?: string; durations?: Record<string, number>; random?: () => number } = {},
) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "vestra-playlist-features-")));
  const host = new TestHost();
  host.tools.handle("ffprobe", probeTable(options.durations));
  const { stop } = await startPlugin((plugin) => activate(plugin, { random: options.random ?? (() => 0) }), {
    directory: new URL("..", import.meta.url),
    host,
    ...(options.data ? { dataDirectory: options.data } : {}),
  });
  const panel = host.mounted();
  const app = {
    host,
    root,
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    names: () => app.rows().map((row) => row.label.replace(/^\d+\. /, "")),
    prop: (name: string) => panel.props[name],
    command: (name: string) => host.command(`playlist.${name}`),
    osd: () => host.osd.at(-1),
    opens: () => host.sent("source.submit").map((call) => call.intent as unknown as Intent),
    emit: async (callback: string, ...args: Json[]) => {
      panel.emit(callback, ...args);
      await host.settle();
    },
    run: async (name: string) => {
      await app.command(name).run();
      await host.settle(20);
    },
    press: async (binding: string, panelFocus = true) => {
      await host.press(binding, { panel: panelFocus });
      await host.settle(20);
    },
    click: (index: number, modifiers: { ctrl?: boolean; shift?: boolean } = {}) =>
      app.emit("select-row", app.rows()[index]!.rowId, !!modifiers.ctrl, !!modifiers.shift),
    /** Write files (relative to the test folder) and return their absolute paths. */
    files: async (names: string[], content = "x") => {
      const paths: string[] = [];
      for (const name of names) {
        const path = join(root, name);
        await mkdir(join(path, ".."), { recursive: true });
        await writeFile(path, content);
        paths.push(path);
      }
      return paths;
    },
    addFiles: async (paths: string[]) => {
      host.dialogAnswers.push(paths);
      await app.run("add");
    },
    end: () => host.setSource({ phase: "ended", terminalReason: "natural" }),
    stop,
    async close() {
      await stop();
      await rm(root, { recursive: true, force: true });
    },
  };
  return app;
}

test("A03: F2 opens a folder recursively in folder order, appends and plays the first new row", async () => {
  const app = await start();
  try {
    const dir = join(app.root, "show");
    await app.files([
      "show/b2.mkv",
      "show/b10.mkv",
      "show/b1.mp4",
      "show/notes.txt",
      "show/.hidden.mp4",
      "show/sub/s2.mp3",
      "show/sub/s1.mp3",
      "show/sub/sub.srt",
      "show/Player.app/inside.mp4",
      "show/list.m3u8",
    ]);
    await symlink(dir, join(dir, "sub", "loop")); // a symlink cycle ends the walk
    app.host.dialogAnswers.push([dir]);
    await app.press("F2", false);
    assert.deepEqual(app.host.sent("dialogs.open").at(-1), { kind: "openDirectory", title: "打开文件夹" });
    assert.deepEqual(app.names(), ["b1.mp4", "b2.mkv", "b10.mkv", "s1.mp3", "s2.mp3"]);
    assert.equal(app.osd(), "已添加 5 项");
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: join(dir, "b1.mp4"), startPaused: false });
    app.host.setSource({ phase: "ready" });
    await app.host.settle();

    // The same folder again: nothing added, the folder's first file's row plays.
    await app.click(3);
    app.host.dialogAnswers.push([dir]);
    await app.run("open-folder");
    assert.equal(app.rows().length, 5);
    assert.equal(app.osd(), "已添加 0 项，跳过 5 个重复");
    assert.equal(app.opens().length, 2);
    assert.equal(app.opens().at(-1)?.source, join(dir, "b1.mp4"));

    // An empty folder and an unreadable one change nothing.
    await mkdir(join(app.root, "empty"));
    app.host.dialogAnswers.push([join(app.root, "empty")]);
    await app.run("open-folder");
    assert.equal(app.osd(), "文件夹中没有可播放的文件");
    app.host.dialogAnswers.push([join(app.root, "missing")]);
    await app.run("open-folder");
    assert.equal(app.osd(), "无法读取文件夹：文件夹不存在");
    app.host.dialogAnswers.push(null); // cancelled
    await app.run("open-folder");
    assert.equal(app.rows().length, 5);
    assert.equal(app.opens().length, 2);
  } finally {
    await app.close();
  }
});

test("A03 / C04: the folder commands are busy while one runs; a second press opens no second dialog", async () => {
  const app = await start();
  try {
    await app.files(["a/1.mp4"]);
    let answer!: (paths: string[]) => void;
    app.host.handle("dialogs.open", () => new Promise((resolve) => (answer = (paths) => resolve({ cancelled: false, paths }))));
    const first = app.command("open-folder").run();
    await pause(20);
    assert.equal(app.command("open-folder").state.busy, true);
    assert.equal(app.command("add-folder").state.busy, true);
    assert.equal(app.command("list-add-folder").state.busy, true);
    await assert.rejects(app.host.press("F2"), "a busy command is not run");
    assert.equal(app.host.sent("dialogs.open").length, 1);
    answer([join(app.root, "a")]);
    await first;
    await app.host.settle(20);
    app.host.reset("dialogs.open");
    assert.equal(app.command("open-folder").state.busy, false);
    assert.equal(app.rows().length, 1);
  } finally {
    await app.close();
  }
});

test("C04: Ctrl+O in the list adds a folder without playing and selects the first row of an empty list", async () => {
  const app = await start();
  try {
    await app.files(["f/x2.mp4", "f/x1.mp4"]);
    app.host.dialogAnswers.push([join(app.root, "f")]);
    await app.press("Ctrl+O");
    assert.deepEqual(app.host.sent("dialogs.open").at(-1), { kind: "openDirectory", title: "添加文件夹到播放列表" });
    assert.deepEqual(app.names(), ["x1.mp4", "x2.mp4"]);
    assert.equal(app.rows()[0]!.selected, true);
    assert.equal(app.opens().length, 0);
    assert.equal(app.osd(), "已添加 2 项");
  } finally {
    await app.close();
  }
});

test("C07: Ctrl+Delete keeps the selection; Ctrl+T removes rows whose files are gone, never links", async () => {
  const app = await start();
  try {
    const [a, b, c, d, e] = await app.files(["a.mp4", "b.mp4", "c.mp4", "d.mp4", "e.mp4"]);
    await app.addFiles([a!, b!, c!, d!, e!]);
    assert.equal(app.command("remove-unselected").state.enabled, true, "one of five is selected");
    await app.click(1);
    await app.click(3, { ctrl: true });
    await app.press("Ctrl+Delete");
    assert.deepEqual(app.names(), ["b.mp4", "d.mp4"]);
    assert.equal(app.osd(), "已移除 3 项");
    assert.equal(app.command("remove-unselected").state.enabled, false, "everything left is selected");
    assert.equal(app.command("remove-unselected").state.reason, "当前没有可操作项");

    await app.run("link");
    app.host.window("playlist-link").emit("add-link", "https://example.org/v");
    await app.host.settle();
    await rm(d!);
    await app.press("Ctrl+T");
    assert.deepEqual(app.names(), ["b.mp4", "https://example.org/v"]);
    assert.equal(app.osd(), "已移除 1 个无效项");
    await app.press("Ctrl+T");
    assert.equal(app.osd(), "没有无效项");
  } finally {
    await app.close();
  }
});

test("C07: removing the playing row keeps the media playing", async () => {
  const app = await start();
  try {
    const [a, b] = await app.files(["a.mp4", "b.mp4"]);
    await app.addFiles([a!, b!]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    await app.click(1);
    await app.press("Ctrl+Delete");
    assert.deepEqual(app.names(), ["b.mp4"]);
    assert.equal(app.host.source.phase, "ready");
    assert.equal(app.rows().some((row) => row.current), false);
  } finally {
    await app.close();
  }
});

test("C11: sort keys, direction, missing values last, stable order, shuffle and reversing", async () => {
  const app = await start({ durations: { "b.mkv": 30, "a10.mp4": 10, "c.avi": 20 }, random: () => 0 });
  try {
    const [b, a10, c, noext, a2] = await app.files(["z/b.mkv", "a10.mp4", "y/c.avi", "noext", "a2.mp4"]);
    await writeFile(b!, "x".repeat(300));
    await writeFile(a10!, "x".repeat(100));
    await writeFile(c!, "x".repeat(200));
    const day = 86_400;
    await utimes(b!, day, day);
    await utimes(a10!, 3 * day, 3 * day);
    await utimes(c!, 2 * day, 2 * day);
    await utimes(noext!, 5 * day, 5 * day);
    await utimes(a2!, 4 * day, 4 * day);
    await app.addFiles([b!, a10!, c!, noext!, a2!]);
    await app.run("link");
    app.host.window("playlist-link").emit("add-link", "https://example.org/dir/Clip%20A.mp4");
    await app.host.settle(30);
    const link = "https://example.org/dir/Clip%20A.mp4";

    await app.press("Ctrl+3"); // title, natural order
    assert.deepEqual(app.names(), ["a2.mp4", "a10.mp4", "b.mkv", "c.avi", link, "noext"]);
    await app.press("Ctrl+4"); // file name: the link sorts as "Clip A.mp4"
    assert.deepEqual(app.names(), ["a2.mp4", "a10.mp4", "b.mkv", "c.avi", link, "noext"]);
    await app.press("Ctrl+5"); // extension; none first, ties by name
    assert.deepEqual(app.names(), ["noext", "c.avi", "b.mkv", "a2.mp4", "a10.mp4", link]);
    await app.press("Ctrl+6"); // size; the link has none
    assert.deepEqual(app.names().slice(0, 2), ["noext", "a2.mp4"]);
    assert.deepEqual(app.names().slice(2), ["a10.mp4", "c.avi", "b.mkv", link]);
    await app.press("Ctrl+2"); // descending re-sorts by the last key; no value stays last
    assert.equal(app.command("sort-descending").state.checked, true);
    assert.deepEqual(app.names(), ["b.mkv", "c.avi", "a10.mp4", "noext", "a2.mp4", link].map((name, i, all) =>
      // noext and a2.mp4 are both 1 byte: the stable sort keeps their order
      i === 3 ? all[3]! : name,
    ));
    await app.press("Ctrl+7"); // duration descending; unknown last
    assert.deepEqual(app.names().slice(0, 3), ["b.mkv", "c.avi", "a10.mp4"]);
    await app.press("Ctrl+1");
    assert.deepEqual(app.names().slice(0, 3), ["a10.mp4", "c.avi", "b.mkv"]);
    await app.press("Ctrl+8"); // date
    assert.deepEqual(app.names(), ["b.mkv", "c.avi", "a10.mp4", "a2.mp4", "noext", link]);
    await app.press("Ctrl+0"); // folder: the folder's own files, then subfolders by name; links last
    assert.deepEqual(app.names(), ["a2.mp4", "a10.mp4", "noext", "c.avi", "b.mkv", link]);
    await app.run("sort-path");
    assert.deepEqual(app.names().at(-1), link, "https:// sorts after /");
    const before = app.names();
    await app.run("sort-reverse");
    assert.deepEqual(app.names(), [...before].reverse());
    await app.press("Ctrl+9"); // random() = 0: each step swaps with the first row
    assert.deepEqual(app.names(), [...before].reverse().slice(1).concat([...before].reverse()[0]!));
    assert.equal(app.command("sort-ascending").state.checked, true, "shuffle ignores the direction");
  } finally {
    await app.close();
  }
});

test("C11: sorting is disabled below two rows", async () => {
  const app = await start();
  try {
    const [a] = await app.files(["a.mp4"]);
    await app.addFiles([a!]);
    assert.equal(app.command("sort-title").state.enabled, false);
    assert.equal(app.prop("can-sort"), false);
  } finally {
    await app.close();
  }
});

test("C17: the list, titles, selection and settings come back after a restart; nothing is opened", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-data-"));
  let app = await start({ data, durations: { "a.mp4": 61 } });
  try {
    const [a, b] = await app.files(["a.mp4", "b.mp4"]);
    await writeFile(join(app.root, "l.m3u8"), `#EXTM3U\n#EXTINF:12,Named B\n${b}\n`);
    app.host.dialogAnswers.push([join(app.root, "l.m3u8")]);
    await app.run("open-list"); // empty list: no confirmation
    await app.addFiles([a!]);
    await app.run("repeat-all");
    await app.run("open-mode-folder");
    await app.click(1);
    await app.host.settle(1100); // the debounced save
    const saved = JSON.parse(await readFile(join(data, "playlists.json"), "utf8"));
    assert.equal(saved.version, 1);
    assert.deepEqual(saved.settings, {
      remember: true,
      repeat: "all",
      shuffle: false,
      openMode: "folder",
      gapless: false,
      clearOnOpenFolder: false,
    });
    assert.deepEqual(saved.list.rows, [
      { kind: "file", source: b, title: "Named B", duration: 12 },
      { kind: "file", source: a, title: null, duration: 61 },
    ]);
    assert.equal(saved.list.cursor, 1);
    assert.deepEqual(saved.list.selected, [1]);
    await app.stop();

    app = await start({ data, durations: {} });
    assert.deepEqual(app.names(), ["Named B", "a.mp4"]);
    assert.deepEqual(app.rows().map((row) => row.duration), ["00:12", "01:01"], "last known lengths at once");
    assert.equal(app.rows()[1]!.selected, true);
    assert.equal(app.command("repeat-all").state.checked, true);
    assert.equal(app.command("open-mode-folder").state.checked, true);
    assert.equal(app.opens().length, 0);

    // Not remembering: the list goes at once, the settings stay.
    await app.run("remember");
    assert.equal(app.osd(), "不再记住列表");
    const settingsOnly = JSON.parse(await readFile(join(data, "playlists.json"), "utf8"));
    assert.equal(settingsOnly.list, undefined);
    assert.equal(settingsOnly.settings.remember, false);
    await app.stop();
    app = await start({ data });
    assert.equal(app.rows().length, 0);
    assert.equal(app.command("repeat-all").state.checked, true);
    assert.equal(app.command("remember").state.checked, false);
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
  }
});

test("C17: a damaged playlists.json starts empty and is kept as a .bad- copy", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-data-"));
  await writeFile(join(data, "playlists.json"), "{ not json");
  const app = await start({ data });
  try {
    assert.equal(app.rows().length, 0);
    assert.ok((await readdir(data)).some((name) => name.startsWith("playlists.json.bad-")));
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
  }
});

test("C18: opening an m3u8 resolves relative paths, file URLs, titles and links; missing files stay", async () => {
  const app = await start();
  try {
    const [a, b] = await app.files(["media/a.mp4", "b.mp4"]);
    const list = join(app.root, "vlc.m3u8");
    await writeFile(
      list,
      [
        "﻿#EXTM3U",
        "#EXTINF:61,First title",
        "media/a.mp4",
        "#EXTINF:-1,Second",
        `file://${encodeURI(b!)}`,
        "# a comment",
        "",
        "https://example.org/stream",
        "rtsp://camera.local/live",
        "gone.mp4",
        "media/../media/a.mp4",
      ].join("\r\n"),
    );
    app.host.dialogAnswers.push([list]);
    await app.press("F3");
    assert.deepEqual(app.host.sent("dialogs.open").at(-1), { kind: "openFile", title: "打开播放列表文件" });
    assert.deepEqual(app.names(), [
      "First title",
      "Second",
      "https://example.org/stream",
      "rtsp://camera.local/live",
      "gone.mp4",
    ]);
    assert.equal(app.osd(), "已打开列表：vlc（5 项），跳过 1 项");
    assert.equal(app.rows()[0]!.tooltip, a);
    assert.equal(app.rows()[0]!.duration, "01:01");
    assert.equal(app.rows()[4]!.tooltip, `${join(app.root, "gone.mp4")}\n文件不存在`);
    assert.equal(app.opens().length, 0, "opening a list does not play");

    // Not a list file.
    app.host.dialogAnswers.push([a!]);
    await app.run("open-list");
    assert.equal(app.osd(), "不支持的列表格式");
  } finally {
    await app.close();
  }
});

test("C18: a non-empty list is replaced only after confirmation", async () => {
  const app = await start();
  try {
    const [a, b] = await app.files(["a.mp4", "b.mp4"]);
    await writeFile(join(app.root, "x.m3u"), `${b}\n`);
    await app.addFiles([a!]);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    app.host.dialogAnswers.push([join(app.root, "x.m3u")]);
    const opening = app.command("open-list").run();
    await app.host.settle(20);
    const confirm = app.host.window("playlist-confirm");
    assert.equal(confirm.visible, true);
    assert.equal(confirm.props.message, "用 x.m3u 替换当前列表？");
    confirm.emit("cancel");
    await opening;
    await app.host.settle();
    assert.deepEqual(app.names(), ["a.mp4"]);
    app.host.dialogAnswers.push([join(app.root, "x.m3u")]);
    const again = app.command("open-list").run();
    await app.host.settle(20);
    confirm.emit("accept");
    await again;
    await app.host.settle();
    assert.deepEqual(app.names(), ["b.mp4"]);
    assert.equal(app.host.source.phase, "ready", "the playing media continues");
    assert.equal(app.rows()[0]!.current, false);
  } finally {
    await app.close();
  }
});

test("C18: GBK .m3u, PLS round trip, DPL; saving adds .m3u8 and writes relative paths below the list", async () => {
  const app = await start({ durations: { "a.mp4": 61.4 } });
  try {
    const [a] = await app.files(["媒体/a.mp4"]);
    const outside = await mkdtemp(join(tmpdir(), "vestra-playlist-out-"));
    const far = join(outside, "far.mp4");
    await writeFile(far, "x");
    // GBK bytes of the Chinese folder name.
    const gbk = Buffer.concat([Buffer.from([0xc3, 0xbd, 0xcc, 0xe5]), Buffer.from("/a.mp4\n")]);
    await writeFile(join(app.root, "gbk.m3u"), gbk);
    app.host.dialogAnswers.push([join(app.root, "gbk.m3u")]);
    await app.run("open-list");
    assert.equal(app.rows()[0]?.tooltip, a);
    await app.addFiles([far]);
    await app.run("link");
    app.host.window("playlist-link").emit("add-link", "https://example.org/v");
    await app.host.settle(30);

    // No extension: .m3u8 is appended; the default name follows the last opened list.
    app.host.dialogAnswers.push([join(app.root, "list")]);
    await app.press("F2");
    assert.deepEqual(app.host.sent("dialogs.open").at(-1), {
      kind: "saveFile",
      title: "保存播放列表",
      fileName: "gbk.m3u8",
    });
    assert.equal(app.osd(), "已保存列表：list.m3u8");
    const m3u8 = await readFile(join(app.root, "list.m3u8"), "utf8");
    assert.equal(
      m3u8,
      `#EXTM3U\n#EXTINF:61,a.mp4\n媒体/a.mp4\n#EXTINF:-1,far.mp4\n${far}\n#EXTINF:-1,https://example.org/v\nhttps://example.org/v\n`,
    );

    // PLS: save, open, save again: the same bytes.
    app.host.dialogAnswers.push([join(app.root, "one.pls")]);
    await app.run("save-list");
    const first = await readFile(join(app.root, "one.pls"), "utf8");
    assert.match(first, /^\[playlist\]\nFile1=媒体\/a\.mp4\nTitle1=a\.mp4\nLength1=61\n/);
    assert.match(first, /NumberOfEntries=3\nVersion=2\n$/);
    app.host.dialogAnswers.push([join(app.root, "one.pls")]);
    const opening = app.command("open-list").run();
    await app.host.settle(20);
    app.host.window("playlist-confirm").emit("accept");
    await opening;
    await app.host.settle(20);
    assert.deepEqual(app.names(), ["a.mp4", "far.mp4", "https://example.org/v"]);
    app.host.dialogAnswers.push([join(app.root, "two.pls")]);
    await app.run("save-list");
    assert.equal(await readFile(join(app.root, "two.pls"), "utf8"), first);

    // .m3u is written with a BOM.
    app.host.dialogAnswers.push([join(app.root, "bom.m3u")]);
    await app.run("save-list");
    assert.deepEqual([...(await readFile(join(app.root, "bom.m3u"))).subarray(0, 3)], [0xef, 0xbb, 0xbf]);

    // reference player DPL (read only).
    await writeFile(
      join(app.root, "pot.dpl"),
      `DPLHEADER\n2*file*${far}\n1*file*${a}\n1*title*Pot A\n1*duration2*5000\n`,
    );
    app.host.dialogAnswers.push([join(app.root, "pot.dpl")]);
    const dpl = app.command("open-list").run();
    await app.host.settle(20);
    app.host.window("playlist-confirm").emit("accept");
    await dpl;
    await app.host.settle(20);
    assert.deepEqual(app.names(), ["Pot A", "far.mp4"]);
    await rm(outside, { recursive: true, force: true });
  } finally {
    await app.close();
  }
});

test("C18: save is disabled for an empty list", async () => {
  const app = await start();
  try {
    assert.equal(app.command("save-list").state.enabled, false);
    assert.equal(app.command("list-save-list").state.enabled, false);
  } finally {
    await app.close();
  }
});

test("C21: search filters by all words (NFKC, any case), keeps numbers, limits actions to shown rows", async () => {
  const app = await start();
  try {
    const paths = await app.files([
      "Show.S01E01.1080p.mkv",
      "Show.S01E02.720p.mkv",
      "Other.S01.1080P.mkv",
      "show.s02e01.1080p.mkv",
    ]);
    await app.addFiles(paths);
    await app.emit("search-changed", "s01");
    assert.equal(app.rows().length, 4, "debounced");
    await app.host.settle(200);
    assert.equal(app.prop("filtering"), true);
    await app.emit("search-changed", "Ｓ01  1080"); // full-width S
    await app.host.settle(200);
    assert.deepEqual(app.rows().map((row) => row.label), ["1. Show.S01E01.1080p.mkv", "3. Other.S01.1080P.mkv"]);
    assert.equal(app.command("move-down").state.enabled, false);
    await app.click(0);
    assert.equal(app.command("move-down").state.reason, "搜索时不能调整顺序");
    assert.equal(app.prop("can-drag"), false);
    await app.press("Ctrl+A");
    await app.press("Delete");
    await app.emit("search-changed", "");
    assert.deepEqual(app.names(), ["Show.S01E02.720p.mkv", "show.s02e01.1080p.mkv"]);
    assert.equal(app.prop("filtering"), false);

    // Hidden rows lose their selection; no match shows an empty list.
    await app.press("Ctrl+A");
    await app.emit("search-changed", "720");
    await app.host.settle(200);
    assert.deepEqual(app.rows().map((row) => row.selected), [true]);
    await app.emit("search-changed", "nothing");
    await app.host.settle(200);
    assert.equal(app.rows().length, 0);
    await app.emit("search-changed", "s02");
    await app.host.settle(200);
    await app.emit("search-accept"); // Enter: the first match plays
    assert.equal(app.opens().at(-1)?.source, paths[3]);

    // Ctrl+S asks the box for focus; focus keeps the bar shown.
    const before = app.prop("focus-search") as number;
    await app.press("Ctrl+S");
    assert.equal(app.prop("focus-search"), before + 1);
    await app.emit("search-focus", true);
    assert.deepEqual(app.host.mounted().locks, { focused: true, dragging: false, popupOpen: false });
    await app.emit("search-focus", false);
    assert.deepEqual(app.host.mounted().locks, { focused: false, dragging: false, popupOpen: false });
  } finally {
    await app.close();
  }
});

test("C21: Ctrl+Delete under a filter removes only shown unselected rows; the total says 筛选", async () => {
  const app = await start({ durations: { "a1.mp4": 60, "a2.mp4": 30, "b.mp4": 10 } });
  try {
    const paths = await app.files(["a1.mp4", "a2.mp4", "b.mp4"]);
    await app.addFiles(paths);
    await app.host.settle(30);
    assert.equal(app.prop("total"), "01:40");
    await app.emit("search-changed", "a");
    await app.host.settle(200);
    assert.equal(app.prop("total"), "筛选 01:30");
    await app.click(0);
    await app.press("Ctrl+Delete");
    await app.emit("search-changed", "");
    assert.deepEqual(app.names(), ["a1.mp4", "b.mp4"]);
    await app.emit("search-changed", "a");
    await app.host.settle(200);
    await app.emit("search-down");
    assert.equal(app.rows()[0]!.selected, true);
  } finally {
    await app.close();
  }
});

test("C23: Ctrl+F reveals the first selected file of each folder; a missing file shows its folder; links are disabled", async () => {
  const app = await start();
  try {
    const [a, b, c] = await app.files(["one/a.mp4", "one/b.mp4", "two/c.mp4"]);
    await app.addFiles([a!, b!, c!]);
    await app.press("Ctrl+A");
    await app.press("Ctrl+F");
    assert.deepEqual(app.host.system.revealed, [a, c], "one Finder window per folder");
    await rm(a!);
    await app.click(0);
    await app.click(1, { ctrl: true });
    await app.press("Ctrl+F");
    assert.deepEqual(app.host.system.revealed.at(-1), b, "a missing file gives way to the next one in its folder");
    await rm(c!);
    await app.click(2);
    await app.press("Ctrl+F");
    assert.deepEqual(app.host.system.opened, [join(app.root, "two")]);
    await rm(join(app.root, "two"), { recursive: true });
    await app.press("Ctrl+F");
    assert.equal(app.osd(), "找不到文件：c.mp4");

    await app.run("link");
    app.host.window("playlist-link").emit("add-link", "https://example.org/v");
    await app.host.settle();
    assert.equal(app.command("reveal").state.enabled, false);
    assert.equal(app.command("reveal").state.reason, "链接没有本地位置");
  } finally {
    await app.close();
  }
});

test("C23: only the first 20 selected files are shown", async () => {
  const app = await start();
  try {
    const paths = await app.files(Array.from({ length: 22 }, (_, i) => `many/${i}.mp4`));
    await app.addFiles(paths);
    await app.press("Ctrl+A");
    await app.press("Ctrl+F");
    assert.deepEqual(app.host.system.revealed, [paths[0]]);
    assert.equal(app.osd(), "只显示前 20 个文件");
  } finally {
    await app.close();
  }
});

test("A13: similar files of a file another plugin opened are added and its row adopted; the end advances", async () => {
  const app = await start();
  try {
    const [, a2, a3] = await app.files(["dir/A01.mkv", "dir/A02.mkv", "dir/A03.mkv", "dir/B.mkv", "dir/song.mp3"]);
    await app.run("open-mode-similar");
    assert.equal(app.command("open-mode-similar").state.checked, true);
    app.host.openElsewhere(a2!);
    await app.host.settle(30);
    assert.deepEqual(app.names(), ["A01.mkv", "A02.mkv", "A03.mkv"]);
    assert.deepEqual(app.rows().map((row) => row.current), [false, true, false]);
    assert.equal(app.osd(), "已加入同文件夹 3 个文件");
    assert.deepEqual(app.host.sequence, { canNext: true, canPrevious: true });
    assert.equal(app.opens().length, 0, "this plugin opened nothing");

    const ended = { ...app.host.source.stamp };
    app.end();
    app.end(); // a repeated end event of the same playthrough
    await app.host.settle(30);
    assert.equal(app.opens().length, 1);
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: a3, startPaused: false, expected: ended });
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[2]!.current, true);
  } finally {
    await app.close();
  }
});

test("A13: the whole folder; an existing row is adopted without adding; repeat one restarts", async () => {
  const app = await start();
  try {
    const [a1, a2] = await app.files(["dir/A01.mkv", "dir/A02.mkv", "dir/A03.mkv", "dir/B.mkv", "dir/song.mp3"]);
    await app.run("open-mode-folder");
    app.host.openElsewhere(a1!);
    await app.host.settle(30);
    assert.deepEqual(app.names(), ["A01.mkv", "A02.mkv", "A03.mkv", "B.mkv", "song.mp3"]);
    const notices = app.host.osd.length;
    app.host.openElsewhere(a2!);
    await app.host.settle(30);
    assert.equal(app.rows().length, 5);
    assert.equal(app.rows()[1]!.current, true);
    assert.equal(app.host.osd.length, notices, "nothing added, nothing said");

    await app.run("repeat-one");
    const ended = { ...app.host.source.stamp };
    app.end();
    await app.host.settle(30);
    assert.deepEqual(app.opens().at(-1), { kind: "restart", expected: ended });
    assert.equal(app.rows()[1]!.current, true, "a restart keeps the adoption");

    // Another outside source ends the adoption silently; a URL is never handled.
    app.host.openElsewhere("https://example.org/v");
    await app.host.settle(30);
    assert.equal(app.rows().some((row) => row.current), false);
    assert.equal(app.rows().length, 5);
  } finally {
    await app.close();
  }
});

test("A13: \"仅打开该文件\" (default) neither adds nor adopts, even for a listed file; own opens are not handled", async () => {
  const app = await start();
  try {
    const [a1, a2] = await app.files(["dir/A01.mkv", "dir/A02.mkv"]);
    await app.addFiles([a1!]);
    app.host.openElsewhere(a1!);
    await app.host.settle(30);
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows()[0]!.current, false);
    assert.deepEqual(app.host.sequence, { canNext: false, canPrevious: false });

    await app.run("open-mode-folder");
    await app.click(0);
    await app.press("Enter"); // this plugin's own open: no folder is added
    app.host.setSource({ phase: "ready" });
    await app.host.settle(30);
    assert.equal(app.rows().length, 1);
    assert.equal(app.rows()[0]!.current, true);
    void a2;
  } finally {
    await app.close();
  }
});

test("C17: a list above 20 000 rows is refused", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-data-"));
  const rows = Array.from({ length: 20_000 }, (_, i) => ({ kind: "url", source: `https://example.org/${i}`, title: null, duration: null }));
  await writeFile(
    join(data, "playlists.json"),
    JSON.stringify({ version: 1, settings: {}, list: { sort: null, cursor: null, selected: [], rows } }),
  );
  const app = await start({ data });
  try {
    assert.equal(app.rows().length, 20_000);
    const [a] = await app.files(["a.mp4"]);
    await app.addFiles([a!]);
    assert.equal(app.osd(), "已添加 0 项，列表已达上限 20000 项");
    assert.equal(existsSync(join(data, "playlists.json")), true);
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
  }
});

test("B26: \"快速推进到下一个\" is off by default, toggles with a notice and is remembered", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-data-"));
  let app = await start({ data });
  try {
    assert.equal(app.command("fast-advance").state.checked, false);
    assert.deepEqual(app.command("fast-advance").declaration.path, ["播放", "播放列表", "快速推进到下一个"]);
    await app.run("fast-advance");
    assert.equal(app.command("fast-advance").state.checked, true);
    assert.equal(app.osd(), "快速推进到下一个：开");
    await app.stop();
    assert.equal(JSON.parse(await readFile(join(data, "playlists.json"), "utf8")).settings.gapless, true);
    app = await start({ data });
    assert.equal(app.command("fast-advance").state.checked, true);
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
  }
});

test("B26: on, the natural end opens the prepared next row at once, without reading the file again", async () => {
  for (const fast of [false, true]) {
    const app = await start();
    try {
      const [a, b] = await app.files(["a.mp4", "b.mp4"]);
      await app.addFiles([a!, b!]);
      if (fast) await app.run("fast-advance");
      await app.click(0);
      await app.press("Enter");
      app.host.setSource({ phase: "ready" });
      await app.host.settle(30); // the next row is prepared while this one plays
      // b turns into an alias of a after that: only a fresh file check notices.
      await rm(b!);
      await symlink(a!, b!);
      const ended = { ...app.host.source.stamp };
      app.end();
      await app.host.settle(30);
      if (fast) {
        assert.equal(app.opens().length, 2);
        assert.deepEqual(app.opens()[1], { kind: "open", source: b, startPaused: false, expected: ended });
      } else {
        assert.equal(app.opens().length, 1, "the usual path checks the file at the end");
        assert.equal(app.osd(), "文件规范路径与已有项重复，已选择已有项");
      }
    } finally {
      await app.close();
    }
  }
});

test("B26: the prepared row follows list changes; a next file that is gone is skipped with a notice", async () => {
  const app = await start();
  try {
    const [a, b, c, d] = await app.files(["a.mp4", "b.mp4", "c.mp4", "d.mp4"]);
    await app.addFiles([a!, b!, c!, d!]);
    await app.run("fast-advance");
    await app.click(0);
    await app.press("Enter");
    app.host.setSource({ phase: "ready" });
    await app.host.settle(30);
    // Moving b to the bottom makes c the next row: a, c, d, b.
    await app.click(1);
    await app.press("Ctrl+End");
    assert.deepEqual(app.names(), ["a.mp4", "c.mp4", "d.mp4", "b.mp4"]);
    await rm(d!); // gone before c starts, so c's next row is checked without it
    await app.host.settle(30);
    let ended = { ...app.host.source.stamp };
    app.end();
    await app.host.settle(30);
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: c, startPaused: false, expected: ended });

    // The end of c skips d and opens b.
    app.host.setSource({ phase: "ready" });
    await app.host.settle(30);
    ended = { ...app.host.source.stamp };
    app.end();
    await app.host.settle(30);
    assert.ok(app.host.osd.includes("无法打开 d.mp4：文件不存在"));
    assert.deepEqual(app.opens().at(-1), { kind: "open", source: b, startPaused: false, expected: ended });
    assert.equal(app.opens().length, 3, "nothing was submitted for d");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal((app.rows()[2] as ViewRow & { failed: boolean }).failed, true);
    assert.equal(app.rows()[3]!.current, true);
  } finally {
    await app.close();
  }
});

test("C17: ffprobe answers are kept in probe-cache.json and reused after a restart", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-playlist-data-"));
  let app = await start({ data, durations: { "a.mp4": 61 } });
  let first = "";
  try {
    await app.run("remember"); // no saved list: the length can only come from the cache
    const [a] = await app.files(["a.mp4"]);
    await app.addFiles([a!]);
    await app.host.settle(30);
    assert.equal(app.rows()[0]!.duration, "01:01");
    await app.stop();
    first = app.root;
    const cache = JSON.parse(await readFile(join(data, "probe-cache.json"), "utf8"));
    assert.deepEqual(cache.map((entry: [string, number]) => entry[1]), [61]);
    app = await start({ data, durations: {} });
    await app.addFiles([a!]);
    await app.host.settle(30);
    assert.equal(app.rows()[0]!.duration, "01:01");
    assert.equal(app.host.tools.calls.length, 0, "answered from the cache");
  } finally {
    await app.close();
    await rm(data, { recursive: true, force: true });
    if (first) await rm(first, { recursive: true, force: true });
  }
});
