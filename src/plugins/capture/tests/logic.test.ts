import assert from "node:assert/strict";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import type { PlayerState } from "@vestra-player/sdk";
import { Failure, frameName, sampleTimes, uniqueFolder, usable, validate } from "../model.ts";
import type { Spec } from "../model.ts";
import { DEFAULT_TEMPLATES, insertVariable, NAME_LIMIT, normalTemplate, renderName, safeName, TEMPLATE_LIMIT } from "../names.ts";
import { burstCount, burstStep, settingsOf } from "../settings.ts";

const reason = (run: () => unknown) => {
  try {
    run();
  } catch (error) {
    return error instanceof Failure ? error.code : String(error);
  }
  return "";
};
const state = (patch: Partial<PlayerState>) => ({ kind: "media", phase: "ready", source: "/m/a.mp4", duration: 30, position: 2, can: { snapshot: true }, ...patch }) as PlayerState;

test("file-name templates: the defaults name files as before; variables, illegal characters, empty and long templates", () => {
  const now = new Date(2026, 10, 7, 16, 21, 5);
  const values = { source: "/movies/bbb_sunflower.mp4", start: 309.412, now };
  assert.deepEqual(Object.keys(DEFAULT_TEMPLATES), ["image", "burst"], "frames and burst folders only");
  assert.equal(renderName("image", DEFAULT_TEMPLATES.image, values), frameName("/movies/bbb_sunflower.mp4", 309.412, ""), "the snapshot default is today's name");
  assert.equal(renderName("image", DEFAULT_TEMPLATES.image, values), "bbb_sunflower_05.09.412");
  assert.equal(renderName("burst", DEFAULT_TEMPLATES.burst, values), "Capture-2026-11-07 16.21.05");
  assert.equal(renderName("burst", "{name} {start}-{end}", { ...values, end: 318.412 }), "bbb_sunflower 05.09.412-05.18.412", "a burst's first and last frame");
  assert.equal(renderName("image", "{n} {date}_{time} {pos} {end} {start}", { ...values, n: 7 }), "007 2026-11-07_16.21.05 05.09.412 05.09.412 05.09.412");
  assert.equal(renderName("image", "{name}:{nope}/x", values), "bbb_sunflower_{nope}_x", "an unknown variable stays; illegal characters become _");
  assert.equal(renderName("image", "  ", values), "bbb_sunflower_05.09.412", "a blank template uses the default");
  assert.equal(renderName("image", "...", values), "bbb_sunflower_05.09.412", "a name that comes out empty uses the default");
  assert.equal(renderName("image", "{name}", { ...values, source: null }), "capture");
  assert.equal(renderName("image", "{name}", { ...values, source: "https://x.test/v/clip.m3u8?token=1" }), "clip");
  assert.equal(safeName("a\u0001b. "), "a_b");
  // Windows device names, whatever follows the first dot and in any case.
  const devices = [["CON", "CON_"], ["nul", "nul_"], ["Aux.png", "Aux_.png"], ["com1.tar.gz", "com1_.tar.gz"], ["LPT9", "LPT9_"], ["COM¹", "COM¹_"], ["conin$", "conin$_"], ["CONOUT$.txt", "CONOUT$_.txt"], ["console", "console"], ["com10", "com10"], ["prn_1", "prn_1"]];
  for (const [name, safe] of devices) assert.equal(safeName(name!), safe, name);
  assert.equal(renderName("image", "{name}", { ...values, source: "/m/aux.mp4" }), "aux_", "a device name from the source");
  assert.equal(renderName("burst", "NUL", values), "NUL_");
  // Long names: the source's name gives way, the time stays; never over the limit.
  const long = renderName("image", DEFAULT_TEMPLATES.image, { ...values, source: `/m/${"x".repeat(400)}.mp4`, start: 1 });
  assert.equal(long.length, NAME_LIMIT);
  assert.match(long, /^x+_00\.01\.000$/);
  assert.ok(renderName("image", "{date}".repeat(40), values).length <= NAME_LIMIT, "no {name} to shorten: cut at the limit");
  assert.ok(NAME_LIMIT + ".jpeg".length + " (999)".length <= 180);
  assert.equal(normalTemplate("burst", ""), DEFAULT_TEMPLATES.burst);
  assert.equal(normalTemplate("burst", 5), DEFAULT_TEMPLATES.burst);
  assert.equal(normalTemplate("image", "x".repeat(500)).length, TEMPLATE_LIMIT);
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

test("a frame to capture: a ready or ended media source the player can snapshot; a plugin's stream has none", () => {
  assert.equal(usable(state({})), true);
  assert.equal(usable(state({ source: "https://x.test/a.m3u8" })), true, "network sources too");
  assert.equal(usable(state({ phase: "ended" })), true, "a finished file keeps its last frame");
  assert.equal(usable(state({ kind: "stream", source: "producer://1" } as Partial<PlayerState>)), false);
  assert.equal(usable(state({ phase: "loading" })), false);
  assert.equal(usable(state({ can: { snapshot: false } as PlayerState["can"] })), false);
});

test("burst limits: 2–20 frames, 0.1–10 s apart, inside a known duration", () => {
  assert.deepEqual(sampleTimes(1, 3, 0.5, 30), [1, 1.5, 2]);
  assert.equal(reason(() => sampleTimes(1, 1, 1, 30)), "burstLimits");
  assert.equal(reason(() => sampleTimes(1, 21, 1, 30)), "burstLimits");
  assert.equal(reason(() => sampleTimes(1, 3, 0.05, 30)), "burstLimits");
  assert.equal(reason(() => sampleTimes(29, 3, 1, 30)), "burstPastEnd");
  const burst: Spec = { kind: "burst", start: 2, count: 3, step: 1, processed: false, subtitles: false, current: false };
  assert.equal(reason(() => validate(burst, state({ duration: null }))), "needDuration");
  assert.equal(reason(() => validate({ ...burst, kind: "image", count: 1 }, state({ duration: null }))), "", "a frame needs no duration");
  assert.equal(reason(() => validate({ ...burst, kind: "image", count: 1, start: 31 }, state({}))), "outOfRange");
  assert.equal(reason(() => validate(burst, state({ can: { snapshot: false } as PlayerState["can"] }))), "unusable");
  // The settings page's fields: a value is kept only when valid.
  assert.deepEqual(["2", " 20 ", "1", "21", "2.5", "", "x"].map(burstCount), [2, 20, null, null, null, null, null]);
  assert.deepEqual(["0.1", "10", "0.05", "11", "", "abc"].map(burstStep), [0.1, 10, null, null, null, null]);
});

test("settings fall back field by field; the defaults", () => {
  const s = settingsOf({ format: "gif", quality: 300, burst: { count: 50, step: 0.5, processed: "yes" }, templates: { image: "{n}", burst: 3 }, besideMedia: 1 }, "/pictures");
  assert.equal(s.format, "png");
  assert.equal(s.quality, 100);
  assert.deepEqual(s.burst, { count: 10, step: 0.5, processed: false, subtitles: false });
  assert.deepEqual(s.templates, { image: "{n}", burst: DEFAULT_TEMPLATES.burst });
  assert.equal(s.folder, "/pictures");
  assert.deepEqual([s.besideMedia, s.screenSubtitles, s.snapshotEffects], [false, true, false]);
  assert.deepEqual(Object.keys(settingsOf({})).sort(), ["besideMedia", "burst", "folder", "format", "quality", "screenSubtitles", "snapshotEffects", "templates"]);
});

test("settings stored by the plugin with the Capture tab load; the clip and tab fields are ignored", () => {
  const old = {
    format: "jpeg", quality: 80, besideMedia: true, snapshotEffects: true,
    method: "precise", encode: { crf: 20, speed: "fast" }, lastSeconds: 60, page: 2, burstOpen: true, advancedOpen: true, preciseSpeed: 1.5,
    burst: { count: 5, step: 0.5, fromNow: false, processed: true, subtitles: true },
    templates: { image: "{n}", clip: "{name}_{start}-{end}", burst: "B {date}" },
  };
  const s = settingsOf(old, "/pictures");
  assert.deepEqual(s, {
    folder: "/pictures", format: "jpeg", quality: 80, screenSubtitles: true, besideMedia: true,
    burst: { count: 5, step: 0.5, processed: true, subtitles: true },
    templates: { image: "{n}", burst: "B {date}" },
    snapshotEffects: true,
  });
});
