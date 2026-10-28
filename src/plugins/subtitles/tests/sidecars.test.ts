// Same-name subtitle files next to a local video (mpv `sub-auto=exact`): the matching rule and the loading.
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { SIDECAR_LIMIT, folders, sidecars } from "../sidecars.ts";

const directory = new URL("..", import.meta.url);
const pause = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const srt = (text: string) => `1\n00:00:01,000 --> 00:00:02,000\n${text}\n`;
const embedded = { kind: "embedded" as const, id: 1 };

test("matching: the video's name, an optional dotted tag, a subtitle extension; exact name first", () => {
  const entries = [
    "movie.mkv", "movie.zh-Hans.ass", "movie.en.srt", "Movie.SRT", "movie2.srt", "movie (1).srt", "my movie.srt",
    "movie.txt", "movie.nfo", "movie.en.forced.srt", "movie.idx", "movie.vtt", "movie.", "movie..srt", "other.srt",
  ];
  assert.deepEqual(sidecars("movie.mkv", entries), [
    "Movie.SRT", "movie.vtt", "movie.en.forced.srt", "movie.en.srt", "movie.zh-Hans.ass",
  ]);
  assert.deepEqual(sidecars("MOVIE.MP4", ["movie.srt"]), ["movie.srt"], "case-insensitive");
  assert.deepEqual(sidecars("Show.S01E01.mkv", ["Show.S01E01.en.srt", "Show.S01E02.srt", "Show.S01E01.srt"]), [
    "Show.S01E01.srt", "Show.S01E01.en.srt",
  ]);
  // A file that names another video of the folder more closely belongs to that one.
  const folder = ["Movie.mkv", "Movie.Extended.mkv", "Movie.srt", "Movie.Extended.srt", "Movie.Extended.en.srt", "Movie.en.srt"];
  assert.deepEqual(sidecars("Movie.mkv", folder), ["Movie.srt", "Movie.en.srt"]);
  assert.deepEqual(sidecars("Movie.Extended.mkv", folder), ["Movie.Extended.srt", "Movie.Extended.en.srt"]);
  // macOS lists decomposed names; the open path may be composed.
  assert.deepEqual(sidecars("Cafe\u0301.mkv", ["Café.srt"]), ["Café.srt"]);
  assert.deepEqual(sidecars("noext", ["noext.srt", "noext.en.ass"]), ["noext.srt", "noext.en.ass"]);
  assert.deepEqual(sidecars("movie.mkv", []), []);
});

/** A folder with `files` (name → content); the video is `movie.mp4`. */
async function folderWith(files: Record<string, string | Buffer>) {
  const dir = await mkdtemp(join(tmpdir(), "vestra-sidecars-"));
  await writeFile(join(dir, "movie.mp4"), "not a video");
  for (const [name, content] of Object.entries(files)) await writeFile(join(dir, name), content);
  return { dir, video: join(dir, "movie.mp4"), cleanup: () => rm(dir, { recursive: true, force: true }) };
}
/**
 * The player's subtitle limit, which the test host does not model: at most `limit` live tracks per
 * plugin and source (src/host/src/playback/subtitle_state.rs MAX_OWNER_RESOURCES = 4), refused at the commit
 * with the code the player sends (`busy`).
 */
class LimitedHost extends TestHost {
  limit = 4;
  refusals = 0;
  override request<T>(method: string, params?: Parameters<TestHost["request"]>[1]): Promise<T> {
    if (method === "subtitles.commit" && this.subtitles.tracks.filter((t) => t.target.kind === "external").length >= this.limit) {
      this.refusals++;
      return Promise.reject(new HostError("busy", "subtitle resource or counter limit exceeded"));
    }
    return super.request<T>(method, params);
  }
}
/** A host with `video` open and ready (generation 1), optionally with an embedded track selected. */
function hostOn(video: string, withEmbedded = false) {
  const host = new LimitedHost();
  host.openElsewhere(video);
  if (withEmbedded)
    host.subtitles.tracks.push({ target: embedded, title: "English", language: "en", styleSupported: true, styleReason: null });
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: withEmbedded ? embedded : null };
  return host;
}
/** Wait until `count` uploads were sent (the plugin reads the folder and the files itself), then let things settle. */
async function uploads(host: TestHost, count: number) {
  for (let i = 0; i < 200 && host.subtitleLoads.length < count; i++) await host.settle(10);
  await pause(50);
  await host.settle();
}
/** Give a load that should not happen time to happen. */
async function quiet(host: TestHost) {
  await pause(200);
  await host.settle();
}
const text = (host: TestHost) => host.subtitleLoads.map((l) => l.bytes.toString("utf8").split("\n")[2]);

test("opening a local video loads its same-name subtitles, exact name first, and shows the first; no OSD", async () => {
  const files = await folderWith({
    "movie.en.srt": srt("English"),
    "movie.srt": srt("Exact"),
    "MOVIE.zh-Hans.srt": srt("Chinese"),
    "movie2.srt": srt("Wrong prefix"),
    "movie-trailer.srt": srt("Wrong prefix"),
  });
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 2);
    await quiet(host);
    assert.deepEqual(text(host), ["Exact", "English"], "at most 2, exact name first");
    assert.deepEqual(host.subtitleLoads.map((l) => l.select), ["primary", "none"]);
    assert.deepEqual(host.subtitleLoads.map((l) => l.title), ["movie.srt", "movie.en.srt"]);
    assert.equal(host.subtitles.selection.target?.kind, "external");
    assert.deepEqual(host.osd, [], "quiet, like mpv");
    // Room is left under the player's 4 tracks: Open Subtitle… adds a third, then Reload Subtitles
    // (the new track before the old one goes) needs a fourth for a moment.
    host.dialogAnswers.push([join(files.dir, "MOVIE.zh-Hans.srt")]);
    await host.command("load").run();
    await uploads(host, 3);
    assert.equal(text(host).at(-1), "Chinese");
    await host.command("track-1").run(); // back to movie.srt
    await host.settle();
    await writeFile(join(files.dir, "movie.srt"), srt("Exact v2"));
    await host.command("reload").run();
    await uploads(host, 4);
    assert.equal(text(host).at(-1), "Exact v2");
    assert.equal(host.osd.at(-1), "已重载字幕 movie.srt");
    assert.equal(host.refusals, 0);
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("the encoding is detected like Open Subtitle…: GB18030 bytes arrive as UTF-8", async () => {
  // "中文" in GB18030.
  const gb = Buffer.concat([Buffer.from("1\n00:00:01,000 --> 00:00:02,000\n"), Buffer.from([0xd6, 0xd0, 0xce, 0xc4]), Buffer.from("\n")]);
  const files = await folderWith({ "movie.srt": gb });
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    assert.deepEqual(text(host), ["中文"]);
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("an embedded default gives way to the same-name file; another plugin's track or a chosen one does not", async () => {
  const files = await folderWith({ "movie.srt": srt("Exact") });
  const host = hostOn(files.video, true);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    assert.equal(host.subtitleLoads[0]!.select, "primary", "external files are preferred, as in mpv");
    // The user picks the embedded track, then a new file appears and Refresh loads it without switching.
    await host.command("track-1").run();
    await host.settle();
    assert.deepEqual(host.subtitles.selection.target, embedded);
    await writeFile(join(files.dir, "movie.en.srt"), srt("English"));
    await host.command("refresh").run();
    await uploads(host, 2);
    assert.equal(host.subtitleLoads[1]!.select, "none");
    assert.deepEqual(host.subtitles.selection.target, embedded, "the user's choice stays");
    assert.equal(host.osd.at(-1), "字幕轨已刷新。已从视频所在文件夹载入字幕文件：1");
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("no duplicates: repeated events and Refresh load each file once per source; reopening loads it again", async () => {
  const files = await folderWith({ "movie.srt": srt("Exact") });
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    host.setSource({ phase: "ready", position: 5 }); // more player events on the same source
    await host.command("refresh").run();
    await host.command("refresh").run();
    await quiet(host);
    assert.equal(host.subtitleLoads.length, 1);
    assert.equal(host.osd.at(-1), "字幕轨已刷新。");
    // Reopening the same file is a new source without the old tracks: its files load again, once.
    host.openElsewhere(files.video);
    host.subtitles.tracks.length = 0;
    host.subtitles.selection = { stamp: { generation: 2, revision: 1 }, target: null };
    await uploads(host, 2);
    await host.command("refresh").run();
    await quiet(host);
    assert.equal(host.subtitleLoads.length, 2);
    assert.equal(host.subtitles.selection.target?.kind, "external");
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("playlist advance: the next video gets its own files, not the previous one's", async () => {
  const files = await folderWith({ "movie.srt": srt("Movie"), "next.srt": srt("Next"), "next.en.srt": srt("Next EN") });
  await writeFile(join(files.dir, "next.mkv"), "not a video");
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    host.openElsewhere(join(files.dir, "next.mkv"));
    host.subtitles.tracks.length = 0;
    host.subtitles.selection = { stamp: { generation: 2, revision: 1 }, target: null };
    await uploads(host, 3);
    assert.deepEqual(text(host), ["Movie", "Next", "Next EN"]);
    assert.deepEqual(host.subtitleLoads.map((l) => l.select), ["primary", "primary", "none"]);
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("unreadable, image, empty and oversized files are skipped quietly; the rest still load", async () => {
  const files = await folderWith({
    "movie.a.srt": "",
    "movie.b.sub": Buffer.from([0x00, 0x00, 0x01, 0xba, 0, 0, 0, 0]),
    "movie.c.srt": "x".repeat(9 * 1024 * 1024),
    "movie.d.srt": "not a subtitle at all",
    "movie.e.srt": srt("Fine"),
  });
  await mkdir(join(files.dir, "movie.f.srt")); // a folder with a subtitle name
  const host = hostOn(files.video);
  const errors: string[] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => void errors.push(args.join(" "));
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    await pause(200);
    await host.settle();
    assert.deepEqual(text(host), ["Fine"]);
    assert.equal(host.subtitleLoads[0]!.select, "primary", "the first file that loads is shown");
    assert.deepEqual(host.osd, []);
    assert.equal(errors.filter((e) => e.includes("skipped")).length, 5, errors.join("\n"));
  } finally {
    console.error = original;
    await stop();
    await files.cleanup();
  }
});

test("setting off: nothing loads; turning it on loads the current video's files; it is saved", async () => {
  const data = await mkdtemp(join(tmpdir(), "vestra-sidecars-data-"));
  await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, autoLoad: false }));
  const files = await folderWith({ "movie.srt": srt("Exact") });
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "zh-Hans" });
  try {
    await host.command("refresh").run();
    await pause(200);
    await host.settle();
    assert.equal(host.subtitleLoads.length, 0);
    await host.command("settings").run();
    const settings = host.window("subtitle-settings");
    assert.equal(settings.props["auto-load"], false);
    settings.emit("change-auto-load", true);
    await uploads(host, 1);
    assert.equal(host.subtitleLoads.length, 1);
    assert.equal(settings.props["auto-load"], true);
    for (let i = 0; i < 50; i++) {
      const saved = await readFile(join(data, "settings.json"), "utf8").then((t) => JSON.parse(t) as Record<string, unknown>, (): Record<string, unknown> => ({}));
      if (saved.autoLoad === true) break;
      await pause(50);
      if (i === 49) assert.fail("autoLoad was not saved");
    }
  } finally {
    await stop();
    await files.cleanup();
    await rm(data, { recursive: true, force: true });
  }
});

test("URLs, generated streams and a folder that cannot be listed load nothing", async () => {
  for (const source of ["https://example.com/movie.mp4", "file:///C:/movie.mp4", join(tmpdir(), "vestra-missing-folder", "movie.mp4")]) {
    const host = hostOn(source);
    const errors: string[] = [];
    const original = console.error;
    console.error = (...args: unknown[]) => void errors.push(args.join(" "));
    const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
    try {
      await host.command("refresh").run();
      await pause(100);
      await host.settle();
      assert.equal(host.subtitleLoads.length, 0, source);
      assert.equal(host.osd.at(-1), "字幕轨已刷新。", source);
    } finally {
      console.error = original;
      await stop();
    }
  }
});

test("a selection that changes during the load (the player settling its default) is read again, not given up", async () => {
  const files = await folderWith({ "movie.srt": srt("Exact") });
  const host = hostOn(files.video);
  let refused = 0;
  host.handle("subtitles.upload.begin", () => {
    refused++;
    host.reset("subtitles.upload.begin");
    throw new HostError("stale", "selection changed");
  });
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    assert.equal(refused, 1);
    assert.deepEqual(text(host), ["Exact"]);
    assert.equal(host.subtitleLoads[0]!.select, "primary");
  } finally {
    await stop();
    await files.cleanup();
  }
});

test("the player's track limit stops the scan without giving up the file: Refresh loads it once there is room", async () => {
  const files = await folderWith({ "movie.srt": srt("Exact"), "movie.en.srt": srt("English") });
  const host = hostOn(files.video);
  host.limit = 1; // another of this plugin's tracks already takes the room, say
  const errors: string[] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => void errors.push(args.join(" "));
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, 1);
    await quiet(host);
    assert.deepEqual(text(host), ["Exact"]);
    assert.equal(host.refusals, 1, "stops at the first refusal");
    assert.ok(errors.some((e) => e.includes("movie.en.srt not loaded")), errors.join(" | "));
    host.limit = 4;
    await host.command("refresh").run();
    await uploads(host, 2);
    assert.deepEqual(text(host), ["Exact", "English"]);
    assert.equal(host.refusals, 1);
  } finally {
    console.error = original;
    await stop();
    await files.cleanup();
  }
});

test("a folder listing that finishes after a playlist advance does not reset the new file's record", async () => {
  const files = await folderWith({ "movie.srt": srt("Movie"), "next.srt": srt("Next") });
  await writeFile(join(files.dir, "next.mkv"), "not a video");
  const list = folders.list;
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  let calls = 0;
  folders.list = async (folder) => {
    if (calls++ === 0) await held; // the first video's listing is slow
    return list(folder);
  };
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await quiet(host);
    assert.equal(host.subtitleLoads.length, 0);
    host.openElsewhere(join(files.dir, "next.mkv"));
    host.subtitles.tracks.length = 0;
    host.subtitles.selection = { stamp: { generation: 2, revision: 1 }, target: null };
    await uploads(host, 1);
    assert.deepEqual(text(host), ["Next"]);
    release();
    await quiet(host);
    await host.command("refresh").run();
    await quiet(host);
    assert.deepEqual(text(host), ["Next"], "no duplicate, nothing of the first video");
    assert.equal(host.osd.at(-1), "字幕轨已刷新。");
  } finally {
    folders.list = list;
    await stop();
    await files.cleanup();
  }
});

test("an automatic scan cut short by a playlist advance sets no status", async () => {
  const files = await folderWith({ "movie.srt": srt("Movie") });
  await writeFile(join(files.dir, "next.mkv"), "not a video");
  const list = folders.list;
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  folders.list = async (folder) => {
    await held;
    return list(folder);
  };
  const host = hostOn(files.video);
  let refuse: (() => void) | undefined;
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await host.command("settings").run();
    const settings = host.window("subtitle-settings");
    // The upload waits; settle() would wait for it too, so only pause from here on.
    host.handle("subtitles.upload.begin", () => new Promise((_resolve, reject) => (refuse = () => reject(new HostError("stale", "source changed")))));
    release();
    for (let i = 0; i < 200 && !refuse; i++) await pause(10);
    assert.ok(refuse, "the scan is uploading");
    host.openElsewhere(join(files.dir, "next.mkv"));
    host.subtitles.tracks.length = 0;
    host.subtitles.selection = { stamp: { generation: 2, revision: 1 }, target: null };
    host.reset("subtitles.upload.begin");
    refuse();
    await quiet(host);
    assert.equal(settings.props.status, "", "not \"the running operation was cancelled\"");
    assert.deepEqual(host.osd, []);
  } finally {
    folders.list = list;
    await stop();
    await files.cleanup();
  }
});

test("at most the limit is loaded from one folder", async () => {
  const many: Record<string, string> = {};
  for (let i = 0; i < SIDECAR_LIMIT + 3; i++) many[`movie.t${String(i).padStart(2, "0")}.srt`] = srt(`T${i}`);
  const files = await folderWith(many);
  const host = hostOn(files.video);
  const { stop } = await startPlugin(activate, { directory, host, locale: "zh-Hans" });
  try {
    await uploads(host, SIDECAR_LIMIT);
    await pause(100);
    await host.settle();
    assert.equal(host.subtitleLoads.length, SIDECAR_LIMIT);
  } finally {
    await stop();
    await files.cleanup();
  }
});
