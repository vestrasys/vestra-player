import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  abRange, defaultRange, effectiveMethod, encodeOf, estimate, fastContainer, keyframeArgs, lastRange, lastSeconds, parseKeyframes, PRESET_ENCODE, presetOf,
  snapStart, timeline, timelineWindow,
} from "../clip.ts";
import { clippable, Failure, frameName, parseRange, parseTime, tabMode, uniqueFolder } from "../model.ts";
import { DEFAULT_TEMPLATES, insertVariable, normalTemplate, renderName, safeName, TEMPLATE_LIMIT } from "../names.ts";
import { RESULT_LIMIT, savedOf } from "../results.ts";
import { clipExport, hostSupport } from "../seams.ts";
import { settingsOf } from "../settings.ts";
import type { PlayerState } from "@vestra-player/sdk";

const reason = (run: () => unknown) => {
  try {
    run();
  } catch (error) {
    return error instanceof Failure ? error.code : String(error);
  }
  return "";
};

test("time fields: 1:02:03.450, 02:03.450 and seconds; anything else is no time", () => {
  assert.equal(parseTime("1:02:03.450"), 3723.45);
  assert.equal(parseTime("02:03.450"), 123.45);
  assert.equal(parseTime(" 62.5 "), 62.5);
  assert.equal(parseTime("90:00"), 5400, "minutes before the first colon may pass 59");
  assert.equal(parseTime("0"), 0);
  for (const bad of ["", " ", "-1", "1:60", "1:02:60.5", "1.5:00", "1:2:3:4", "abc", "1e3", "NaN", "Infinity", "1:", ":5"]) assert.equal(parseTime(bad), null, bad);
});

test("range parsing: field reasons, the end after the start", () => {
  assert.deepEqual(parseRange("0.1", "2.5"), { start: 0.1, end: 2.5 });
  assert.deepEqual(parseRange("4:02.300", "4:19"), { start: 242.3, end: 259 });
  assert.deepEqual(parseRange("7"), { start: 7, end: undefined });
  for (const [a, b] of [["", 1], [-1, 1], ["NaN", 1], [0, "Infinity"], ["x", "1"]]) assert.equal(reason(() => parseRange(a, b)), "time", `${a}–${b}`);
  for (const [a, b] of [[1, 1], [2, 1], ["4:19", "4:02.3"]]) assert.equal(reason(() => parseRange(a, b)), "endBeforeStart", `${a}–${b}`);
});

test("A-B use: a one-time copy of the player's loop; none without a valid loop", () => {
  assert.deepEqual(abRange({ loop: { a: 242.3, b: 259, count: null } }), { start: 242.3, end: 259 });
  assert.equal(abRange({ loop: null }), null);
  assert.equal(abRange({}), null, "older states without the field");
  assert.equal(abRange({ loop: { a: 5, b: 5, count: 1 } }), null);
  assert.equal(abRange({ loop: { a: Number.NaN, b: 5, count: 1 } }), null);
});

test("Last N s: from N seconds before the position (not before 0); lengths 10/30/60/120, 30 by default", () => {
  assert.deepEqual(lastRange(100, 30), { start: 70, end: 100 });
  assert.deepEqual(lastRange(12, 30), { start: 0, end: 12 }, "clamped at the start of the media");
  assert.deepEqual(lastRange(500, 120), { start: 380, end: 500 });
  assert.equal(reason(() => lastRange(0, 30)), "tooEarly");
  assert.equal(reason(() => lastRange(0.05, 30)), "tooEarly");
  assert.equal(reason(() => lastRange(null, 30)), "tooEarly");
  assert.deepEqual([10, 30, 60, 120, 45, "x", undefined].map(lastSeconds), [10, 30, 60, 120, 30, 30, 30]);
});

test("Export Clip… starts at the position for 10 s, shorter at the end of the media", () => {
  assert.deepEqual(defaultRange(2, 30), { start: 2, end: 12 });
  assert.deepEqual(defaultRange(25, 30), { start: 25, end: 30 });
  assert.deepEqual(defaultRange(null, null), { start: 0, end: 10 });
});

test("Fast: keyframes from ffprobe's packet flags, the cut starts on the last at or before the start", () => {
  const stdout = "238.000000,K__\n239.001000,___\n240.000000,K_\n242.002000,__\n244.000000,K__\r\n\nbad,K\n240.000000,K__\n";
  const keys = parseKeyframes(stdout);
  assert.deepEqual(keys, [238, 240, 244]);
  assert.equal(snapStart(keys, 242.3), 240);
  assert.equal(snapStart(keys, 240), 240, "a start on a keyframe stays");
  assert.equal(snapStart(keys, 237), null, "none known before it");
  const args = keyframeArgs("/m/a.mkv", 228.3, 264);
  assert.deepEqual(args.slice(args.indexOf("-read_intervals"), args.indexOf("-read_intervals") + 2), ["-read_intervals", "228.300%264.000"]);
  assert.ok(args.includes("packet=pts_time,flags") && args.at(-1) === "/m/a.mkv");
  assert.ok(!args.includes("-show_frames"), "packets only: nothing is decoded");
});

test("Fast container: MP4 when the streams fit, MKV otherwise, unknown without the video codec", () => {
  assert.equal(fastContainer("h264", "aac"), "mp4");
  assert.equal(fastContainer("HEVC", null), "mp4");
  assert.equal(fastContainer("vc1", "aac"), "mkv");
  assert.equal(fastContainer("h264", "dts"), "mkv");
  assert.equal(fastContainer(null, "aac"), "");
});

test("the method: Fast only while the player can copy streams (R1); Precise otherwise", () => {
  assert.equal(effectiveMethod("fast", false), "precise");
  assert.equal(effectiveMethod("fast", true), "fast");
  assert.equal(effectiveMethod("precise", true), "precise");
  const support = hostSupport({} as never);
  assert.deepEqual(Object.values(support), [false, false, false, false, false, false], "SDK 1.6.8 offers none of R1–R6");
  assert.equal(hostSupport({} as never, { fastCut: true }).fastCut, true);
  // Until R1 and R6: today's re-encode whatever is asked.
  assert.deepEqual(clipExport("/o/a.mp4", 9, "fast", PRESET_ENCODE.small), { format: "mp4", destination: "/o/a.mp4", end: 9 });
});

test("the small timeline: about 30 s around the clip, ratios of what was typed and what is written", () => {
  assert.deepEqual(timelineWindow(100, 110, 1000), { from: 90, to: 120 });
  assert.deepEqual(timelineWindow(2, 12, 30), { from: 0, to: 22 });
  assert.deepEqual(timelineWindow(0, 100, 100), { from: 0, to: 100 }, "a long clip: a little room, inside the media");
  const line = timeline({ start: 242.3, end: 259 }, { start: 240, end: 259 }, [236, 238, 240, 244, 300], 250, 1000, 240);
  assert.equal(line.shown, true);
  assert.ok(line.gota < line.wanta && line.wantb === line.gotb, "the written range starts earlier");
  assert.equal(line.key, line.gota);
  assert.ok(line.head > line.wanta && line.head < line.wantb);
  assert.equal(line.ticks.split("M").length - 1, 4, "the keyframe outside the window is left out");
  assert.equal(timeline({ start: 1, end: 2 }, { start: 1, end: 2 }, [], 500, 1000, null).head, -1, "a playhead outside the window");
});

test("Precise presets: High quality is today's encoding; any change is Custom; stored options fall back field by field", () => {
  assert.deepEqual(PRESET_ENCODE.high, { crf: 18, speed: "faster", height: 0, fps: 0, audio: 192 });
  assert.equal(presetOf(PRESET_ENCODE.small), "small");
  assert.equal(presetOf({ ...PRESET_ENCODE.high, crf: 19 }), "custom");
  assert.deepEqual(encodeOf({ crf: 99, speed: "warp", height: 720, fps: 30, audio: 1 }), { crf: 30, speed: "faster", height: 720, fps: 30, audio: 192 });
  assert.deepEqual(encodeOf(null), PRESET_ENCODE.high);
});

test("estimates: Fast is the source's share of its file; Precise a range, a measured speed narrows the time", () => {
  const fast = estimate({ method: "fast", length: 19, fileBytes: 1_000_000_000, duration: 400, encode: PRESET_ENCODE.high, secondsPerSecond: null, height: null });
  assert.deepEqual([fast.sizelow, fast.sizehigh, fast.timelow], [47.5, 47.5, 1]);
  const precise = estimate({ method: "precise", length: 19, fileBytes: 1_000_000_000, duration: 400, encode: PRESET_ENCODE.high, secondsPerSecond: null, height: null });
  assert.ok(precise.sizelow < 47.5 && precise.sizehigh > 47.5);
  assert.ok(precise.timehigh > precise.timelow);
  const smaller = estimate({ method: "precise", length: 19, fileBytes: 1_000_000_000, duration: 400, encode: { ...PRESET_ENCODE.high, crf: 24 }, secondsPerSecond: null, height: null });
  assert.ok(Math.abs(smaller.sizehigh * 2 - precise.sizehigh) < 0.2, "6 more on the quality number about halves the size");
  const measured = estimate({ method: "precise", length: 10, fileBytes: null, duration: null, encode: PRESET_ENCODE.high, secondsPerSecond: 2, height: null });
  assert.deepEqual([measured.sizehigh, measured.timelow, measured.timehigh], [0, 16, 25]);
});

test("file-name templates: the defaults name files as before; variables, illegal characters, empty and long templates", () => {
  const now = new Date(2026, 10, 7, 16, 21, 5);
  const values = { source: "/movies/bbb_sunflower.mp4", start: 309.412, now };
  assert.equal(renderName("image", DEFAULT_TEMPLATES.image, values), frameName("/movies/bbb_sunflower.mp4", 309.412, ""), "the snapshot default is today's name");
  assert.equal(renderName("image", DEFAULT_TEMPLATES.image, values), "bbb_sunflower_05.09.412");
  assert.equal(renderName("clip", DEFAULT_TEMPLATES.clip, { ...values, start: 242.3, end: 259 }), "bbb_sunflower_04.02.300-04.19.000");
  assert.equal(renderName("burst", DEFAULT_TEMPLATES.burst, values), "Capture-2026-11-07 16.21.05");
  assert.equal(renderName("image", "{n} {date}_{time} {pos} {end} {start}", { ...values, n: 7 }), "007 2026-11-07_16.21.05 05.09.412 05.09.412 05.09.412");
  assert.equal(renderName("image", "{name}:{nope}/x", values), "bbb_sunflower_{nope}_x", "an unknown variable stays; illegal characters become _");
  assert.equal(renderName("image", "  ", values), "bbb_sunflower_05.09.412", "a blank template uses the default");
  assert.equal(renderName("image", "...", values), "bbb_sunflower_05.09.412", "a name that comes out empty uses the default");
  assert.equal(renderName("image", "{name}", { ...values, source: null }), "capture");
  assert.equal(renderName("image", "{name}", { ...values, source: "https://x.test/v/clip.m3u8?token=1" }), "clip");
  assert.equal(safeName("a\u0001b. "), "a_b");
  assert.equal(normalTemplate("clip", ""), DEFAULT_TEMPLATES.clip);
  assert.equal(normalTemplate("clip", 5), DEFAULT_TEMPLATES.clip);
  assert.equal(normalTemplate("clip", "x".repeat(500)).length, TEMPLATE_LIMIT);
  assert.equal(insertVariable("{name}_", "{pos}"), "{name}_{pos}");
  assert.equal(insertVariable("{name}", "{evil}"), "{name}", "only the listed variables");
});

test("a burst folder is never reused: name, name (1), …", async () => {
  const parent = await mkdtemp(join(tmpdir(), "capture-folders-"));
  try {
    const first = await uniqueFolder(join(parent, "Capture-2026-11-07 16.21.05"));
    const second = await uniqueFolder(join(parent, "Capture-2026-11-07 16.21.05"));
    assert.equal(second, `${first} (1)`);
    assert.deepEqual((await readdir(parent)).sort(), ["Capture-2026-11-07 16.21.05", "Capture-2026-11-07 16.21.05 (1)"]);
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});

test("tab modes: local everything, network frames only, a plugin's stream nothing, none without a frame", () => {
  const state = (patch: Partial<PlayerState>) => ({ kind: "media", phase: "ready", source: "/m/a.mp4", duration: 30, position: 2, can: { snapshot: true }, ...patch }) as PlayerState;
  assert.equal(tabMode(state({})), "local");
  assert.equal(clippable(state({})), true);
  assert.equal(tabMode(state({ source: "https://x.test/a.m3u8" })), "network");
  assert.equal(clippable(state({ source: "https://x.test/a.m3u8" })), false);
  assert.equal(tabMode(state({ kind: "stream", source: "producer://1" })), "stream");
  assert.equal(tabMode(state({ phase: "loading" })), "none");
  assert.equal(tabMode(state({ can: { snapshot: false } as PlayerState["can"] })), "none");
  assert.equal(clippable(state({ duration: null })), false, "a clip needs a known duration");
  assert.equal(tabMode(state({ phase: "ended" })), "local", "a finished file keeps its last frame");
});

test("settings fall back field by field; new fields have their defaults", () => {
  const s = settingsOf({ format: "gif", method: "turbo", encode: { crf: 20 }, burst: { count: 50, step: 0.5, fromNow: "no" }, templates: { image: "{n}", clip: 3 }, lastSeconds: 45, preciseSpeed: -1 }, "/pictures");
  assert.equal(s.format, "png");
  assert.equal(s.method, "fast", "Fast is the default (applies once the player can copy streams)");
  assert.deepEqual(s.encode, { ...PRESET_ENCODE.high, crf: 20 });
  assert.deepEqual(s.burst, { count: 10, step: 0.5, fromNow: true, processed: false, subtitles: false });
  assert.deepEqual(s.templates, { image: "{n}", clip: DEFAULT_TEMPLATES.clip, burst: DEFAULT_TEMPLATES.burst });
  assert.equal(s.lastSeconds, 30);
  assert.equal(s.preciseSpeed, null);
  assert.equal(s.folder, "/pictures");
  assert.deepEqual([s.burstOpen, s.advancedOpen, s.settingsOpen], [false, false, false]);
});

test("stored results: valid ones only, newest first, at most twenty", () => {
  const item = (i: number) => ({ id: `r${i}`, kind: "clip", path: `/o/${i}.mp4`, finished: i, method: "fast", bytes: 5, count: 1, start: 1, end: 2, thumb: "../../evil.jpg" });
  const list = savedOf({ list: [...Array.from({ length: 25 }, (_, i) => item(i)), { id: "x", kind: "song", path: "/a", finished: 1 }, null, 7] });
  assert.equal(list.length, RESULT_LIMIT);
  assert.equal(list[0]!.id, "r24");
  assert.equal(list[0]!.thumb, "evil.jpg", "a thumbnail is a file name inside the data directory");
  assert.deepEqual(savedOf(null), []);
  assert.deepEqual(savedOf({ list: [{ ...item(1), method: "lossy", count: -3 }] })[0]!.method, "");
});
