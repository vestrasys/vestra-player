import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { MESSAGE_LIMIT, startPlugin, TestHost, UI_VALUE_LIMITS } from "@vestra-player/sdk/testing";
import { settleFiles } from "./settle.ts";
import activate from "../main.ts";
import { PERSIST_CHECK_MS, Store } from "../store.ts";

// 0.2.0 (large lists P3): the panel gets every shown row, each with a stable `id`, and draws
// them with a `ListView`. These tests check what the plugin sends and does; what the list does
// with the pointer (drag with edge scrolling, double-click, the insertion marker) is Slint
// code, checked by the source test at the end and by hand in the player.

type ViewRow = {
  id: string;
  number: number;
  label: string;
  duration: string;
  selected: boolean;
  current: boolean;
  thumb: Json;
};
const pause = (ms = 0) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * The row limits of the player with large lists P3 (≥ 20,000 rows per array, about 15
 * values per playlist row). Until the SDK's limits follow, a test with more rows than the
 * current limits take raises the test host's (`UI_VALUE_LIMITS`, which it enforces) for its
 * own run. The 1 MiB protocol line (`MESSAGE_LIMIT`) stays: it bounds one write anyway.
 */
async function withP3Limits(run: () => Promise<void>) {
  const limits = UI_VALUE_LIMITS as { values: number; arrayRows: number };
  const before = { values: limits.values, arrayRows: limits.arrayRows };
  limits.arrayRows = Math.max(before.arrayRows, 20_000);
  limits.values = Math.max(before.values, 20_000 * 16);
  try {
    await run();
  } finally {
    Object.assign(limits, before);
  }
}

/**
 * `count` empty media files in folders of 100, the playlist plugin, and a record of every
 * panel write (the value the panel shows after it) and of every write the host refused.
 */
async function start(count = 1000, compact = true) {
  const root = await mkdtemp(join(tmpdir(), "vestra-playlist-large-"));
  const files: string[] = [];
  for (let folder = 0; folder < count / 100; folder++) {
    const dir = join(root, `Season ${String(folder).padStart(2, "0")}`);
    await mkdir(dir);
    for (let i = 0; i < 100; i++) {
      const file = join(dir, `Show S${folder}E${String(i).padStart(3, "0")} 第${i}集.mkv`);
      await writeFile(file, "");
      files.push(file);
    }
  }
  return launch({ root, files, compact });
}

async function launch(options: { root: string; files: string[]; compact: boolean; data?: string }) {
  const { root, files } = options;
  const host = new TestHost();
  settleFiles(host);
  let probes = 0;
  host.tools.handle("ffprobe", async () => {
    probes++;
    await pause();
    return { stdout: JSON.stringify({ format: { duration: "61" } }) };
  });
  const writes: { property: string; value: Json; bytes: number }[] = [];
  const refused: string[] = [];
  const request = host.request.bind(host);
  host.request = (async (method: string, params: Record<string, Json> = {}) => {
    let result;
    try {
      result = await request(method, params);
    } catch (error) {
      if (method === "ui.setMount") refused.push(`${String(params.property)}: ${(error as Error).message}`);
      throw error;
    }
    // The rows the panel opens with (a remembered list): a write of its own here.
    const rows = (params.properties as Record<string, Json> | undefined)?.rows;
    if (method === "ui.mount" && Array.isArray(rows) && rows.length)
      writes.push({ property: "rows", value: structuredClone(rows), bytes: Buffer.byteLength(JSON.stringify(params)) });
    // The value the panel shows after the write: a row patch (SDK 1.5.44) carries operations.
    if (method === "ui.setMount") {
      const property = String(params.property);
      const value = "patch" in params ? (host.mounted().props[property] as Json) : (params.value as Json);
      writes.push({ property, value: structuredClone(value), bytes: Buffer.byteLength(JSON.stringify(params)) });
    }
    return result;
  }) as typeof host.request;
  const { stop } = await startPlugin((plugin) => activate(plugin), {
    directory: new URL("..", import.meta.url),
    host,
    ...(options.data ? { dataDirectory: options.data } : {}),
  });
  const panel = host.mounted();
  // These tests count rows of 20 px: thumbnails off (the bottom bar's toggle).
  if (options.compact) {
    panel.emit("toggle-thumbnails");
    await host.settle();
  }
  const app = {
    host,
    panel,
    files,
    root,
    writes,
    refused,
    probes: () => probes,
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    count: () => app.rows().length,
    /** Writes of `rows` so far. */
    rowWrites: () => writes.filter((write) => write.property === "rows").length,
    until: async (check: () => boolean, what: string) => {
      for (const deadline = Date.now() + 30_000; !check(); await host.settle(5))
        if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
      await host.settle();
    },
    /** The list reports its scroll (wheel, scroll bar, edge scrolling while dragging) and an 800 px height. */
    scroll: async (y: number) => {
      panel.emit("list-area", y, 800, 31);
      await host.settle();
    },
    emit: async (callback: string, ...args: Json[]) => {
      panel.emit(callback, ...args);
      await host.settle();
    },
    async close() {
      await stop();
      await rm(root, { recursive: true, force: true });
    },
  };
  return app;
}

/** Each row's encoding by id: rows that kept theirs are left alone by the player's row patches. */
const byId = (rows: ViewRow[]) => new Map(rows.map((row) => [row.id, JSON.stringify(row)]));

test("1000 files: the whole list goes to the panel, one row per item with a stable id; durations are read on screen only", async (t) => {
  const app = await start();
  try {
    app.host.dialogAnswers.push([app.root]);
    const started = performance.now();
    await app.host.command("playlist.add-folder").run();
    await app.until(() => app.count() === 1000, "the folder to be added");
    const added = performance.now() - started;
    assert.ok(added < 5000, `adding 1000 files took ${added.toFixed(0)} ms`);
    const rows = app.rows();
    assert.equal(new Set(rows.map((row) => row.id)).size, 1000, "one id per row");
    const whole = app.writes.filter((write) => write.property === "rows" && (write.value as Json[]).length === 1000)[0]!;
    t.diagnostic(`one write of 1000 file rows: ${whole.bytes} bytes (${(whole.bytes / 1000).toFixed(0)} per row)`);
    // The list numbers rows by their index; the label is the title.
    assert.deepEqual(
      rows.slice(0, 2).map((row) => [row.number, row.label]),
      [
        [0, "Show S0E000 第0集.mkv"],
        [0, "Show S0E001 第1集.mkv"],
      ],
    );

    // Only the rows on screen get their durations (50 before the list reports its height;
    // 41 in an 800 px list); nothing else is probed.
    await app.until(() => app.rows().slice(0, 50).every((row) => row.duration === "01:01"), "the first screen");
    assert.equal(app.rows()[50]!.duration, "");
    assert.equal(app.probes(), 50);
    await app.scroll(700 * 20);
    await app.until(() => app.rows().slice(700, 741).every((row) => row.duration === "01:01"), "the shown durations");
    assert.equal(app.rows()[699]!.duration, "", "rows above the view are not read");
    assert.ok(app.probes() <= 50 + 41, `${app.probes()} ffprobe runs`);
    assert.match(String(app.panel.props.total), /\+$/, "the total is open while lengths are unknown");

    // Scrolling over rows already read sends no rows (the player draws the list it has):
    // rows 0–49 and 700–740 are read, 41 rows are on screen.
    const sent = app.rowWrites();
    const ids = byId(app.rows());
    await app.scroll(0);
    await app.scroll(8 * 20);
    await app.scroll(700 * 20 + 10);
    await app.scroll(700 * 20);
    assert.equal(app.rowWrites(), sent, "no rows written for a scroll");
    // Scrolled to rows 30–70: the durations read change those rows only.
    await app.scroll(30 * 20);
    await app.until(() => app.rows()[70]!.duration === "01:01", "the durations of rows 50–70");
    const changed = app.rows().flatMap((row, at) => (ids.get(row.id) === JSON.stringify(row) ? [] : [at]));
    assert.deepEqual(changed, Array.from({ length: 21 }, (_, i) => 50 + i), "rows 50–70 changed");

    // Selecting and playing a row far down the list, then the next one.
    await app.scroll(700 * 20);
    const target = app.rows()[710]!;
    await app.emit("select-row", target.id, false, false);
    await app.emit("play-row", target.id);
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[710]!.current, true);
    await app.host.step("next");
    await app.host.settle();
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[711]!.current, true, "next plays the following row");

    assert.ok(app.probes() < 200, `${app.probes()} ffprobe runs for 1000 rows`);
    assert.deepEqual(app.refused, [], "every write is taken");
  } finally {
    await app.close();
  }
});

test("drag reorder past the view: a drop after the list scrolled moves the dragged rows; every other row keeps its encoding", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 1000, "the files to be added");
    await app.scroll(0);
    // Rows 3 and 5 selected (Ctrl), the drag starts on row 3 at the top of the list.
    const [third, fifth] = [app.rows()[2]!.id, app.rows()[4]!.id];
    await app.emit("select-row", third, false, false);
    await app.emit("select-row", fifth, true, false);
    await app.emit("dragging", true);
    // Edge scrolling carries the list far down (the list reports each step), then the drop:
    // before row 901 of the order the list showed while dragging.
    for (let y = 0; y <= 880 * 20; y += 60 * 20) await app.scroll(y);
    await app.scroll(880 * 20);
    const before = byId(app.rows());
    const ids = app.rows().map((row) => row.id);
    await app.emit("drop-row", third, 900);
    await app.emit("dragging", false);
    const after = app.rows();
    const expected = ids.filter((id) => id !== third && id !== fifth);
    expected.splice(898, 0, third, fifth);
    assert.deepEqual(after.map((row) => row.id), expected, "both selected rows moved, in order");
    assert.ok(after[898]!.selected && after[899]!.selected);
    // Rows keep their encodings (the number is the list's, not the row's): on player 0.6.0
    // the write is two `move` operations, not 1000 rewritten rows.
    const rewritten = after.filter((row) => before.get(row.id) !== JSON.stringify(row));
    assert.deepEqual(rewritten, [], "no row changed but its place");
    // A drop while a search filters does nothing (the list does not drag then, C21).
    app.panel.emit("search-changed", "第5集");
    await pause(200);
    await app.until(() => app.count() === 10, "the search");
    assert.equal(app.panel.props["can-drag"], false);
    await app.emit("drop-row", app.rows()[1]!.id, 0);
    app.panel.emit("search-changed", "");
    await app.until(() => app.count() === 1000, "the search to clear");
    assert.deepEqual(app.rows().map((row) => row.id), expected);
    assert.deepEqual(app.refused, []);
  } finally {
    await app.close();
  }
});

test("selection over 1000 rows: Shift and Ctrl clicks far apart, keys, select all; each row says whether it is selected", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 1000, "the files to be added");
    await app.scroll(0);
    const id = (at: number) => app.rows()[at]!.id;
    const selected = () => app.rows().flatMap((row, at) => (row.selected ? [at] : []));
    await app.emit("select-row", id(10), false, false);
    await app.scroll(500 * 20);
    await app.emit("select-row", id(520), false, true); // Shift: the range
    assert.deepEqual(selected(), Array.from({ length: 511 }, (_, i) => 10 + i));
    await app.emit("select-row", id(300), true, false); // Ctrl: out of the range
    assert.equal(selected().length, 510);
    assert.ok(!selected().includes(300));
    await app.emit("select-row", id(999), false, false);
    assert.deepEqual(selected(), [999]);
    // Keys: ↑ with Shift grows the range from the cursor; the cursor is scrolled into view.
    const asked = app.panel.props["scroll-request"] as number;
    await app.emit("select-step", -3, true);
    assert.deepEqual(selected(), [996, 997, 998, 999]);
    assert.ok((app.panel.props["scroll-request"] as number) > asked);
    assert.equal(app.panel.props["scroll-to-y"], 996 * 20 + 20 - 800, "the cursor row becomes the last row in view");
    await app.host.press("Ctrl+A", { panel: true });
    await app.host.settle();
    assert.equal(selected().length, 1000);
    assert.deepEqual(app.refused, []);
  } finally {
    await app.close();
  }
});

test("double-click while rows update: the row's id plays it, wherever it is by then", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 1000, "the files to be added");
    /** The source of the last open. */
    const opened = () => (app.host.sent("source.submit").at(-1)?.intent as { source?: string } | undefined)?.source;
    // The list reports its height: durations arrive and rewrite rows on screen meanwhile.
    app.panel.emit("list-area", 0, 800, 31);
    const target = app.rows()[5]!;
    app.panel.emit("select-row", target.id, false, false);
    app.panel.emit("play-row", target.id);
    await app.host.settle();
    assert.equal(opened(), app.files[5]);
    // Rows above it removed between the clicks: the id still names the row.
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    const other = app.rows()[11]!;
    await app.emit("select-row", app.rows()[0]!.id, false, false);
    await app.host.press("Delete", { panel: true });
    await app.host.settle();
    assert.equal(app.rows()[10]!.id, other.id, "one row up");
    await app.emit("play-row", other.id);
    assert.equal(opened(), app.files[11]);
    // An id that is gone (a row removed while the second click was on its way) plays nothing.
    const submits = app.host.sent("source.submit").length;
    await app.emit("play-row", "row-999999");
    assert.equal(app.host.sent("source.submit").length, submits);
  } finally {
    await app.close();
  }
});

test("1000 files with thumbnails: the list shows as fast as without; ffmpeg runs only for the rows near the view; pictures go in the rows near it", async () => {
  /** Time until the folder's 1000 rows are shown, and the app (thumbnails on or off). */
  const addFolder = async (compact: boolean) => {
    const app = await start(1000, compact);
    let made = 0;
    app.host.tools.handle("ffmpeg", async () => {
      made++;
      await pause(5);
      return { stdout: Buffer.from([0xff, 0xd8, 0xff, 0xd9]) };
    });
    await app.scroll(0); // the list has its size: 800 px
    app.host.dialogAnswers.push([app.root]);
    const started = performance.now();
    await app.host.command("playlist.add-folder").run();
    await app.until(() => app.count() === 1000, "the folder to be added");
    return { app, took: performance.now() - started, made: () => made };
  };
  const off = await addFolder(true);
  await off.app.close();
  const on = await addFolder(false);
  try {
    const { app } = on;
    // Measured on the 2026-10 Windows laptop: both ~0.3 s; the bound leaves room for a loaded machine.
    assert.ok(on.took < Math.max(2 * off.took, off.took + 1000), `thumbnails on ${on.took.toFixed(0)} ms, off ${off.took.toFixed(0)} ms`);
    assert.ok(on.took < 5000);
    // 800 px of 62 px rows: 13 on screen (14 with the one cut), 3 more below.
    await app.until(() => on.made() === 17, "the first screen's thumbnails");
    await app.host.settle(100);
    assert.equal(on.made(), 17, "nothing beyond the rows near the view");
    assert.equal(app.probes(), 17, "durations: the same rows");
    const picture = (row: ViewRow) => {
      const path = app.host.imagePath(row.thumb);
      return !!path && !path.endsWith("placeholder.png");
    };
    await app.until(() => app.rows().filter(picture).length === 17, "the pictures");
    // Far down: only the rows near the new view; pictures only within 20 rows of the view.
    await app.scroll(700 * 62);
    await app.until(() => on.made() === 17 + 20, "the new screen's thumbnails");
    await app.host.settle(100);
    assert.equal(on.made(), 37);
    const shown = app.rows().flatMap((row, at) => (picture(row) ? [at] : []));
    assert.ok(shown.every((at) => at >= 700 - 20 && at < 700 + 14 + 20), `pictures in rows ${shown.join(", ")}`);
    assert.equal(shown.length, 20);
    // One reference per picture (the same one in every write), plus the placeholder.
    const images = new Set(app.host.images.map((image) => image.ref)).size;
    assert.equal(images, 38, "37 pictures and the placeholder");
    assert.deepEqual(app.refused, []);
  } finally {
    await on.app.close();
  }
});

test("1000 files: search numbers rows by their place in the list; select all and delete is quick", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 1000, "the files to be added");
    app.panel.emit("search-changed", "第99集");
    await pause(200);
    await app.until(() => app.count() === 10, "the search");
    assert.deepEqual([app.rows()[1]!.number, app.rows()[1]!.label], [200, basename(app.files[199]!)]);
    app.panel.emit("search-changed", "");
    await app.until(() => app.count() === 1000, "the search to clear");
    assert.ok(app.rows().every((row) => row.number === 0), "numbered by the list again");

    const started = performance.now();
    await app.host.press("Ctrl+A", { panel: true });
    await app.host.settle();
    await app.host.press("Delete", { panel: true });
    await app.until(() => app.count() === 0, "the rows to go");
    const took = performance.now() - started;
    assert.ok(took < 2000, `removing 1000 rows took ${took.toFixed(0)} ms`);
    assert.deepEqual(app.refused, []);
  } finally {
    await app.close();
  }
});

test("1000 files: the list opens on the playing row, centred (F6, its tab, detached); a row playing out of view is shown", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push([app.root]);
    await app.host.command("playlist.add-folder").run();
    await app.until(() => app.count() === 1000, "the folder to be added");
    await app.scroll(0); // the list reports its height: 40 rows of 20 px
    const props = app.panel.props;
    const request = () => props["scroll-request"] as number;
    /** The scroll that puts row `index` (0-based) in the middle of the 800 px list, clamped. */
    const middle = (index: number) => Math.max(0, Math.min(index * 20 + 10 - 400, 1000 * 20 - 800));
    const play = async (index: number) => {
      await app.scroll(middle(index));
      app.panel.emit("play-row", app.rows()[index]!.id);
      await app.host.settle();
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      assert.equal(app.rows()[index]?.current, true, `row ${index + 1} plays`);
    };
    /** Close and reopen the list (F6 twice); the new panel reports where it starts (`show-y`). */
    const reopen = async () => {
      await app.host.command("playlist.visibility").run();
      await app.host.settle();
      assert.equal(app.host.layout.sidebar?.effective, false);
      const start = props["show-y"] as number;
      await app.host.command("playlist.visibility").run();
      await app.host.settle();
      await app.scroll(start);
      return start;
    };

    // Item 900 of 1000 plays; the user scrolls back to the top and closes the list.
    await play(899);
    await app.scroll(0);
    let asked = request();
    const start = await reopen();
    assert.equal(start, middle(899), "the new panel starts with row 900 in the middle");
    assert.equal(props["scroll-to-y"], middle(899));
    assert.ok(request() > asked, "and is asked to scroll there once it shows");
    assert.equal(app.rows()[899]?.current, true);
    assert.equal(899 * 20 + 10 - start, 400, "its middle is the list's middle");

    // Clamped at the list's end and start.
    await play(998);
    await app.scroll(0);
    assert.equal(await reopen(), 1000 * 20 - 800, "the last row: the list ends at the bottom");
    await play(1);
    await app.scroll(10_000);
    assert.equal(await reopen(), 0, "the second row: the list starts at the top");

    // Another plugin's tab, then the playlist's tab again.
    await play(600);
    await app.scroll(0);
    app.host.mountOther("sidebar", "example.other", { title: "Other" });
    app.host.selectTab("example.other");
    await app.host.settle();
    assert.equal(props["show-y"], middle(600), "hidden behind another tab: centred for the next panel");
    asked = request();
    app.host.selectTab("vestra.playlist");
    await app.host.settle();
    assert.ok(request() > asked);
    assert.equal(props["scroll-to-y"], middle(600));
    await app.scroll(middle(600));

    // Detached into its own window: a new panel, centred again.
    await app.scroll(0);
    await pause(1100); // past the panel's settling time: the scroll above is the user's
    asked = request();
    await app.host.request("layout.set", { slot: "sidebar", detached: true });
    app.host.emit("window.state", {});
    await app.host.settle();
    assert.ok(request() > asked, "detaching re-centres the playing row");
    assert.equal(props["scroll-to-y"], middle(600));
    await app.scroll(middle(600));
    await pause(1100);

    // While shown: the next row is in view, no scroll; a row playing off screen is centred.
    asked = request();
    const subs = () => app.host.sent("source.submit").length;
    // `settle()` waits for host calls only: the next row's open follows the plugin's own file
    // check (realpath), so wait for it before the source becomes ready.
    await app.host.step("next");
    await app.until(() => subs() === 5, "the next row's open");
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[601]?.current, true);
    assert.equal(request(), asked, "the next row is in view: the list stays");
    app.panel.emit("play-row", app.rows()[500]!.id);
    await app.host.settle();
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(props["scroll-to-y"], middle(500), "a playing row out of view is centred");
    assert.deepEqual(app.refused, []);
  } finally {
    await app.close();
  }
});

/**
 * One write of the whole list must fit the 1 MiB protocol line the SDK's test host (and the
 * player) take: 4000 rows do not (about 300 bytes a row with the placeholder picture).
 * Large lists P3 has to raise it (or split the first write); until then this test waits.
 */
const P3_LINE = MESSAGE_LIMIT >= 4 * 1024 * 1024 ? false : "needs the large lists P3 player and SDK: one write of 4000 rows is over the 1 MiB protocol line";

test("4000 rows (P3 limits): sent whole once, then scrolling, selecting and moving write only what changed", { skip: P3_LINE }, async (t) => {
  await withP3Limits(async () => {
    // A remembered list of 4000 links (short sources: one write of 4000 rows must stay under
    // the 1 MiB protocol line; see the measurement below).
    const data = await mkdtemp(join(tmpdir(), "vestra-playlist-large-data-"));
    const rows = Array.from({ length: 4000 }, (_, i) => ({ kind: "url", source: `https://e.org/${i}`, title: null, duration: null }));
    await writeFile(
      join(data, "playlists.json"),
      JSON.stringify({ version: 1, settings: { thumbnails: false }, list: { sort: null, cursor: null, selected: [], rows } }),
    );
    const root = await mkdtemp(join(tmpdir(), "vestra-playlist-large-"));
    const app = await launch({ root, files: [], compact: false, data });
    try {
      await app.scroll(0);
      await app.until(() => app.count() === 4000, "the remembered list");
      const whole = app.writes.filter((write) => write.property === "rows").at(-1)!;
      t.diagnostic(`one write of 4000 link rows: ${whole.bytes} bytes (${(whole.bytes / 4000).toFixed(0)} per row)`);
      assert.ok(whole.bytes < MESSAGE_LIMIT);
      const sent = app.rowWrites();
      for (const y of [1000, 20_000, 79_200, 40_000]) await app.scroll(y);
      assert.equal(app.rowWrites(), sent, "scrolling writes no rows (links have no duration to read)");
      // Ctrl+End after selecting the first row: it moves to the end, the others keep their encodings.
      const first = app.rows()[0]!;
      const before = byId(app.rows());
      await app.emit("select-row", first.id, false, false);
      await app.host.press("Ctrl+End", { panel: true });
      await app.host.settle();
      assert.equal(app.rows().at(-1)!.id, first.id);
      const rewritten = app.rows().filter((row) => before.get(row.id) !== JSON.stringify(row));
      assert.deepEqual(rewritten.map((row) => row.id), [first.id], "the moved row only (now selected)");
      assert.equal(app.panel.props["scroll-to-y"], 4000 * 20 - 800, "the list follows the row to the end");
      // Persistence takes the 4000-row snapshot only when the list changed.
      await pause(1300);
      const saved = JSON.parse(await readFile(join(data, "playlists.json"), "utf8")) as { list: { rows: { source: string }[] } };
      assert.equal(saved.list.rows.at(-1)!.source, "https://e.org/0");
      assert.deepEqual(app.refused, []);
    } finally {
      await app.close();
      await rm(data, { recursive: true, force: true });
    }
  });
});

test("persistence: no snapshot of the list for updates that change nothing saved", async () => {
  // `Store.snapshot` covers every row: count its calls.
  const snapshot = Store.snapshot;
  let snapshots = 0;
  Store.snapshot = (...args: Parameters<typeof snapshot>) => {
    snapshots++;
    return snapshot(...args);
  };
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 1000, "the files to be added");
    await app.scroll(0);
    await app.until(() => app.rows().slice(0, 41).every((row) => row.duration === "01:01"), "the durations");
    await pause(PERSIST_CHECK_MS + 200); // the burst's snapshot
    snapshots = 0;
    // Scrolls within read rows and position ticks: updates, nothing saved changes.
    for (let i = 0; i < 10; i++) {
      await app.scroll(i * 20);
      app.host.setSource({ phase: "ready", position: i });
      await app.host.settle();
    }
    await pause(PERSIST_CHECK_MS + 200);
    assert.equal(snapshots, 0, "no snapshot without a change");
    await app.emit("select-row", app.rows()[3]!.id, false, false);
    await app.emit("select-row", app.rows()[4]!.id, true, false);
    await pause(PERSIST_CHECK_MS + 200);
    assert.equal(snapshots, 1, "one snapshot after a burst of changes");
  } finally {
    Store.snapshot = snapshot;
    await app.close();
  }
});

test("sidebar.slint draws the rows with a ListView over `rows` (no ScrollView with a `for`), pointer handling at list level", async () => {
  const source = await readFile(new URL("../ui/sidebar.slint", import.meta.url), "utf8");
  const code = source
    .split("\n")
    .map((line) => line.replace(/\/\/.*$/, ""))
    .join("\n");
  assert.match(code, /import \{[^}]*\bListView\b[^}]*\} from "std-widgets\.slint"/);
  assert.doesNotMatch(code, /\bScrollView\b/, "no ScrollView");
  // The one `for` over the rows is the ListView's own child.
  const fors = [...code.matchAll(/\bfor\s+(\w+)(?:\[\w+\])?\s+in\s+([\w.-]+)\s*:/g)];
  assert.deepEqual(fors.map((m) => m[2]), ["PlaylistBridge.rows"]);
  const list = code.indexOf("list := ListView {");
  assert.ok(list >= 0);
  // Between the ListView's start and the `for`: its own properties only, no element.
  const head = code.slice(list + "list := ListView {".length, code.indexOf("for row", list));
  assert.doesNotMatch(head, /(?:^|\n)\s*(?:[\w-]+\s*:=\s*)?[A-Z][\w-]*\s*\{/, "the for is the ListView's direct child");
  // No hand-made window: no first row, row count property or padding rows.
  assert.doesNotMatch(code, /first-row|row-count:\s*PlaylistBridge\.row-count|\bthumbs\b/);
  // Rows hold no TouchArea (nor a ToolTipArea, which has one): a row element can be destroyed
  // in the middle of a press. One TouchArea around the list takes press, move, release and
  // double-click, and maps the pointer to a row by the fixed row height and the list's scroll.
  const rowStart = source.indexOf("for row[index] in PlaylistBridge.rows");
  const rowEnd = source.indexOf("// Insertion marker", rowStart);
  assert.ok(rowStart >= 0 && rowEnd > rowStart);
  const row = source.slice(rowStart, rowEnd).replace(/\/\/.*$/gm, "");
  assert.doesNotMatch(row, /TouchArea|ToolTipArea/);
  assert.match(code, /pointer := TouchArea \{/);
  for (const handler of ["pointer-event(event)", "moved =>", "double-clicked =>"])
    assert.ok(code.indexOf(handler, code.indexOf("pointer := TouchArea")) < list, `${handler} is the list's`);
  assert.match(code, /Math\.floor\(\(y - list\.content-y\) \/ root\.row-height\)/);
  // Every row has the fixed height of its style (the pointer and A15 drop mapping rely on it).
  assert.match(row, /^\s*height: root\.row-height;$/m);
});
