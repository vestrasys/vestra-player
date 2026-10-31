import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import type { Json } from "@vestra-player/sdk";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import { settleFiles } from "./settle.ts";
import activate from "../main.ts";
import { WINDOW_MARGIN, WINDOW_ROWS, windowStart } from "../scroll.ts";

type ViewRow = { rowId: string; label: string; duration: string; selected: boolean; current: boolean };
const pause = (ms = 0) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * What the player accepts as one property value (host `ui_values.rs`): at most
 * 1024 array rows, 4096 values, 1 MiB of text including field names.
 */
function hostRefuses(value: Json): string | null {
  let nodes = 0,
    text = 0;
  const walk = (v: Json): string | null => {
    if (++nodes > 4096) return "more than 4096 values";
    if (typeof v === "string") text += Buffer.byteLength(v);
    else if (Array.isArray(v)) {
      if (v.length > 1024) return `${v.length} array rows`;
      for (const item of v) {
        const why = walk(item);
        if (why) return why;
      }
    } else if (v && typeof v === "object")
      for (const [key, item] of Object.entries(v)) {
        text += Buffer.byteLength(key);
        const why = walk(item as Json);
        if (why) return why;
      }
    return text > 1024 * 1024 ? "more than 1 MiB of text" : null;
  };
  return walk(value);
}

/** 3000 empty media files in 30 folders, the playlist plugin, and a record of every panel write. */
async function start(count = 3000, compact = true) {
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
  const host = new TestHost();
  settleFiles(host);
  let probes = 0;
  host.tools.handle("ffprobe", async () => {
    probes++;
    await pause();
    return { stdout: JSON.stringify({ format: { duration: "61" } }) };
  });
  const writes: { property: string; value: Json }[] = [];
  const request = host.request.bind(host);
  host.request = (async (method: string, params: Record<string, Json> = {}) => {
    const result = await request(method, params);
    // The value the panel shows after the write: a row patch (SDK 1.5.44) carries operations.
    if (method === "ui.setMount") {
      const property = String(params.property);
      writes.push({ property, value: "patch" in params ? (host.mounted().props[property] as Json) : (params.value as Json) });
    }
    return result;
  }) as typeof host.request;
  const { stop } = await startPlugin((plugin) => activate(plugin), { directory: new URL("..", import.meta.url), host });
  const panel = host.mounted();
  // These tests count rows of 20 px: thumbnails off (the bottom bar's toggle).
  if (compact) {
    panel.emit("toggle-thumbnails");
    await host.settle();
  }
  const app = {
    host,
    panel,
    files,
    root,
    writes,
    probes: () => probes,
    rows: () => (panel.props.rows ?? []) as unknown as ViewRow[],
    count: () => panel.props["row-count"] as number,
    first: () => panel.props["first-row"] as number,
    until: async (check: () => boolean, what: string) => {
      for (const deadline = Date.now() + 30_000; !check(); await host.settle(5))
        if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
      await host.settle();
    },
    scroll: async (y: number) => {
      panel.emit("list-area", y, 800, 31);
      await host.settle();
    },
    async close() {
      await stop();
      await rm(root, { recursive: true, force: true });
    },
  };
  return app;
}

test("windowStart: keeps the window while the view is well inside it, else centres it", () => {
  assert.equal(windowStart(0, 0, 800, 150), 0, "short lists are sent whole");
  assert.equal(windowStart(0, 0, 800, 3000), 0);
  // 41 rows on screen: at row 100 the view ends 59 rows before the window's end (200): kept.
  assert.equal(windowStart(0, 100 * 20, 800, 3000), 0);
  // Within the margin of the end: centred on the view.
  const moved = windowStart(0, 130 * 20, 800, 3000);
  assert.equal(moved, 130 - Math.floor((WINDOW_ROWS - 41) / 2));
  assert.ok(130 - moved >= WINDOW_MARGIN && moved + WINDOW_ROWS - (130 + 41) >= WINDOW_MARGIN);
  assert.equal(windowStart(moved, 140 * 20, 800, 3000), moved, "small scrolls keep it");
  assert.equal(windowStart(1000, 2990 * 20, 800, 3000), 3000 - WINDOW_ROWS, "clamped to the end");
  assert.equal(windowStart(2800, 2959 * 20, 800, 3000), 2800, "the end of the list needs no margin");
  assert.equal(windowStart(500, 0, 800, 3000), 0, "a jump to the top");
});

test("3000 files: every panel write fits the player's limits, scrolling moves a 200-row window", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push([app.root]);
    const started = performance.now();
    await app.host.command("playlist.add-folder").run();
    await app.until(() => app.count() === 3000, "the folder to be added");
    const added = performance.now() - started;
    assert.ok(added < 5000, `adding 3000 files took ${added.toFixed(0)} ms`);
    assert.equal(app.rows().length, WINDOW_ROWS);
    assert.equal(app.first(), 0);
    assert.deepEqual(
      app.rows().slice(0, 2).map((row) => row.label),
      ["1. Show S0E000 第0集.mkv", "2. Show S0E001 第1集.mkv"],
    );

    // Only the rows on screen get their durations (50 before the list reports its height;
    // 41 in an 800 px list); nothing else is probed.
    await app.until(() => app.rows().slice(0, 50).every((row) => row.duration === "01:01"), "the first screen");
    assert.equal(app.rows()[50]!.duration, "");
    assert.equal(app.probes(), 50);
    await app.scroll(1500 * 20);
    assert.ok(app.first() > 1300 && app.first() <= 1500, `window from ${app.first()}`);
    const at = 1500 - app.first();
    assert.equal(app.rows()[at]!.label, `1501. ${basename(app.files[1500]!)}`);
    await app.until(() => app.rows().slice(at, at + 41).every((row) => row.duration === "01:01"), "the shown durations");
    assert.equal(app.rows()[at - 1]!.duration, "", "rows above the view are not read");
    assert.ok(app.probes() <= 50 + 41, `${app.probes()} ffprobe runs`);
    assert.match(String(app.panel.props.total), /\+$/, "the total is open while lengths are unknown");

    // Small scrolls inside the window keep it (no rows are resent for them).
    const first = app.first();
    await app.scroll(1500 * 20 + 40);
    await app.scroll(1500 * 20 + 80);
    assert.equal(app.first(), first);
    // Selecting and playing a row far down the list.
    const target = app.rows()[10]!;
    app.panel.emit("select-row", target.rowId, false, false);
    await app.host.settle();
    app.panel.emit("play-row", target.rowId);
    await app.host.settle();
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows().find((row) => row.rowId === target.rowId)?.current, true);
    await app.host.step("next");
    await app.host.settle();
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(app.rows()[11]!.current, true, "next plays the following row");

    assert.ok(app.probes() < 200, `${app.probes()} ffprobe runs for 3000 rows`);
    for (const { property, value } of app.writes) assert.equal(hostRefuses(value), null, `${property} is refused`);
    assert.ok(Math.max(...app.writes.filter((w) => w.property === "rows").map((w) => (w.value as Json[]).length)) <= WINDOW_ROWS);
  } finally {
    await app.close();
  }
});

test("2000 files with thumbnails: the list shows as fast as without; ffmpeg runs only for the rows near the view", async () => {
  /** Time until the folder's 2000 rows are shown, and the app (thumbnails on or off). */
  const addFolder = async (compact: boolean) => {
    const app = await start(2000, compact);
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
    await app.until(() => app.count() === 2000, "the folder to be added");
    return { app, took: performance.now() - started, made: () => made };
  };
  const off = await addFolder(true);
  await off.app.close();
  const on = await addFolder(false);
  try {
    const { app } = on;
    // Measured on the 2026-10 Windows laptop: both ~0.6 s; the bound leaves room for a loaded machine.
    assert.ok(on.took < Math.max(2 * off.took, off.took + 1000), `thumbnails on ${on.took.toFixed(0)} ms, off ${off.took.toFixed(0)} ms`);
    assert.ok(on.took < 5000);
    // 800 px of 62 px rows: 13 on screen (14 with the one cut), 3 more below.
    await app.until(() => on.made() === 17, "the first screen's thumbnails");
    await app.host.settle(100);
    assert.equal(on.made(), 17, "nothing beyond the rows near the view");
    assert.equal(app.probes(), 17, "durations: the same rows");
    const thumbs = app.panel.props.thumbs as Json[];
    assert.equal(thumbs.length, WINDOW_ROWS, "one picture per row sent");
    // Far down: only the rows near the new view.
    await app.scroll(1500 * 62);
    await app.until(() => on.made() === 17 + 20, "the new screen's thumbnails");
    await app.host.settle(100);
    assert.equal(on.made(), 37);
    for (const { property, value } of app.writes) assert.equal(hostRefuses(value), null, `${property} is refused`);
    // One reference per picture (the same one in every write), plus the placeholder.
    const images = new Set(app.host.images.map((image) => image.ref)).size;
    assert.equal(images, 38, "37 pictures and the placeholder");
  } finally {
    await on.app.close();
  }
});

test("3000 files: search numbers rows by their place in the list; select all and delete is quick", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push(app.files);
    await app.host.command("playlist.add").run();
    await app.until(() => app.count() === 3000, "the files to be added");
    app.panel.emit("search-changed", "第99集");
    await pause(200);
    await app.until(() => app.count() === 30, "the search");
    assert.equal(app.rows()[1]!.label, `200. ${basename(app.files[199]!)}`);
    app.panel.emit("search-changed", "");
    await app.until(() => app.count() === 3000, "the search to clear");

    const started = performance.now();
    await app.host.press("Ctrl+A", { panel: true });
    await app.host.settle();
    await app.host.press("Delete", { panel: true });
    await app.until(() => app.count() === 0, "the rows to go");
    const took = performance.now() - started;
    assert.ok(took < 2000, `removing 3000 rows took ${took.toFixed(0)} ms`);
  } finally {
    await app.close();
  }
});

test("3000 files: the list opens on the playing row, centred (F6, its tab, detached); a row playing out of view is shown", async () => {
  const app = await start();
  try {
    app.host.dialogAnswers.push([app.root]);
    await app.host.command("playlist.add-folder").run();
    await app.until(() => app.count() === 3000, "the folder to be added");
    await app.scroll(0); // the list reports its height: 40 rows of 20 px
    const props = app.panel.props;
    const request = () => props["scroll-request"] as number;
    /** The scroll that puts row `index` (0-based) in the middle of the 800 px list, clamped. */
    const middle = (index: number) => Math.max(0, Math.min(index * 20 + 10 - 400, 3000 * 20 - 800));
    /** The row shown at `index`, from the rows sent (`null`: not in the 200-row window). */
    const sent = (index: number) => app.rows()[index - app.first()] ?? null;
    const play = async (index: number) => {
      await app.scroll(middle(index));
      app.panel.emit("play-row", sent(index)!.rowId);
      await app.host.settle();
      app.host.setSource({ phase: "ready" });
      await app.host.settle();
      assert.equal(sent(index)?.current, true, `row ${index + 1} plays`);
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

    // Item 2500 of 3000 plays; the user scrolls back to the top and closes the list.
    await play(2499);
    await app.scroll(0);
    let asked = request();
    const start = await reopen();
    assert.equal(start, middle(2499), "the new panel starts with row 2500 in the middle");
    assert.equal(props["scroll-to-y"], middle(2499));
    assert.ok(request() > asked, "and is asked to scroll there once it shows");
    assert.equal(sent(2499)?.current, true, "the playing row is among the rows sent");
    assert.equal(2499 * 20 + 10 - start, 400, "its middle is the list's middle");
    // The 40 rows on screen are in the window (no empty rows when the panel appears).
    assert.ok(app.first() <= start / 20 && app.first() + WINDOW_ROWS >= start / 20 + 41);

    // Clamped at the list's end and start.
    await play(2998);
    await app.scroll(0);
    assert.equal(await reopen(), 3000 * 20 - 800, "the last row: the list ends at the bottom");
    assert.equal(sent(2998)?.current, true);
    await play(1);
    await app.scroll(30_000);
    assert.equal(await reopen(), 0, "the second row: the list starts at the top");

    // Another plugin's tab, then the playlist's tab again.
    await play(1800);
    await app.scroll(0);
    app.host.mountOther("sidebar", "example.other", { title: "Other" });
    app.host.selectTab("example.other");
    await app.host.settle();
    assert.equal(props["show-y"], middle(1800), "hidden behind another tab: centred for the next panel");
    asked = request();
    app.host.selectTab("vestra.playlist");
    await app.host.settle();
    assert.ok(request() > asked);
    assert.equal(props["scroll-to-y"], middle(1800));
    await app.scroll(middle(1800));

    // Detached into its own window: a new panel, centred again.
    await app.scroll(0);
    await pause(1100); // past the panel's settling time: the scroll above is the user's
    asked = request();
    await app.host.request("layout.set", { slot: "sidebar", detached: true });
    app.host.emit("window.state", {});
    await app.host.settle();
    assert.ok(request() > asked, "detaching re-centres the playing row");
    assert.equal(props["scroll-to-y"], middle(1800));
    await app.scroll(middle(1800));
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
    assert.equal(sent(1801)?.current, true);
    assert.equal(request(), asked, "the next row is in view: the list stays");
    const far = app.first(); // in the window, 60+ rows above the view
    app.panel.emit("play-row", sent(far)!.rowId);
    await app.host.settle();
    app.host.setSource({ phase: "ready" });
    await app.host.settle();
    assert.equal(props["scroll-to-y"], middle(far), "a playing row out of view is centred");
  } finally {
    await app.close();
  }
});
