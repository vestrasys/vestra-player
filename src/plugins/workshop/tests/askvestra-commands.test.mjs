// Ask Vestra, commands (PROPOSAL §9 step 5): the model gets the palette's
// command table without personal rows, a `command` step is checked against
// that table before any step runs, and runs through `commands.run` with the
// request's ticket, the first step naming the rest of the plan (`then`) so
// the player's approval sheet asks once for all of it (Ask Vestra UX
// PROPOSAL step 3, P2/P4); Don't Run, a disabled or busy command and a
// third-party command end or continue the request as the PROPOSAL's flows
// say. Commands only, no Undo (user ruling 2026-11-11). A local mock
// model only. How a request ended is an end state of the palette's status
// (`status.state`), which the player shows in its status strip.
//
// The TestHost lists only the player's core commands and this plugin's own,
// so the tests that need other plugins' rows answer `commands.list` and
// `commands.run` themselves (`table`), the way the player does: a plan with
// a dangerous row asks once (the queued dialog answer), a disabled or busy
// one fails with its reason, an unknown id fails `missing`.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { HostError } from "@vestra-player/sdk/testing";
import { dataDirectory, server, startAi } from "./fixture.mjs";
import { guard, shown, track } from "./lead-status.mjs";

guard(test);

const plan = (...steps) => ({ toolCalls: [{ name: "run_steps", args: { steps } }] });
const command = (id) => ({ kind: "command", id });
const CONTINUE = { confirmed: true, checked: false };
const DONT_RUN = { confirmed: false, checked: false };

/**
 * The end states the plugin sent (`palette.lead.set` with `status.state`
 * done, failed or note: the player's status strip shows each once), as
 * "state: text" in English.
 */
const ends = (host) =>
  host
    .sent("palette.lead.set")
    .filter((p) => p.status?.state && p.status.state !== "working")
    .map((p) => `${p.status.state}: ${shown(host, p.status.text)}`);

/** A row of the palette's table as `commands.list` gives it. */
const row = (id, label, extra = {}) => ({
  id,
  label,
  labelSource: label,
  title: label.split(": ").at(-1),
  category: label.split(": ")[0],
  path: [],
  place: "",
  keys: [],
  plugin: "Vestra Player",
  builtin: true,
  enabled: true,
  checkable: false,
  radio: false,
  group: "domain",
  busy: false,
  dangerous: false,
  ...extra,
});

/**
 * Answers `commands.list` and `commands.run` with `rows`, as the player
 * would (SDK 1.7.16's TestHost): a run
 * with `then` is judged with its plan, and when any of it asks the
 * approval sheet asks once (one queued dialog answer, recorded in `asks`
 * as the plan's ids); Run keeps the rest of the plan for the ticket, each
 * later run of a planned id uses up one entry without asking, anything
 * else asks on its own. `rows` may change between runs (`busy`).
 * Returns the runs, the sheets asked (`runs.asks`) and a way to drop the
 * kept plan (`runs.forget`); `ran(params)` is called after each run.
 */
function table(host, rows, ran = () => {}) {
  const runs = [];
  const asks = [];
  let kept = null;
  host.handle("commands.list", () => ({ revision: 1, commands: rows }));
  host.handle("commands.run", (p) => {
    const find = (id) => {
      const found = rows.find((each) => each.id === id);
      if (!found) throw new HostError("missing", `no command ${id}`);
      return found;
    };
    const found = find(p.id);
    if (!found.enabled) throw new HostError("disabled", found.reason ?? `${found.label} is disabled`);
    if (found.busy) throw new HostError("busy", found.reason ?? `${found.label} is busy`);
    const then = p.then ?? null;
    let plan = [p.id];
    let ask;
    if (then !== null) {
      kept = null;
      plan = [p.id, ...then];
      ask = plan.map(find).some((each) => each.dangerous);
    } else {
      const at = kept?.indexOf(p.id) ?? -1;
      if (at >= 0) kept.splice(at, 1);
      ask = at < 0 && found.dangerous;
    }
    if (ask) {
      asks.push(plan);
      const answer = host.dialogAnswers.shift();
      if (!answer?.confirmed) {
        runs.push({ id: p.id, declined: true });
        return { ran: false, declined: true };
      }
      if (then !== null) kept = [...then];
    }
    runs.push({ id: p.id, ticket: p.ticket, step: p.step, of: p.of, ...(then !== null ? { then } : {}) });
    ran(p);
    return { ran: true };
  });
  // Not enumerable: `assert.deepEqual(runs, [...])` compares the runs alone.
  Object.defineProperty(runs, "asks", { value: asks });
  Object.defineProperty(runs, "forget", { value: () => (kept = null) });
  return runs;
}

/** The plugin as a built-in, the mock model set up, a file playing at 1×. */
async function ready(reply, options = {}) {
  const mock = await server(reply);
  const ai = await startAi({ compat: mock.url, builtin: true, ...options });
  track(ai.host);
  ai.host.openElsewhere("/media/Secret Holiday Film.mkv");
  ai.host.setSource({ duration: 600, position: 30, paused: false });
  await ai.host.settle();
  return { ...ai, mock, end: () => ends(ai.host).at(-1), system: () => mock.chats.at(-1).body.messages[0].content };
}

test("a command of the player runs through commands.run with the request's ticket; a plain result line", async (t) => {
  const ai = await ready(() => plan(command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  const ticket = await host.runPaletteLead("mute");
  const { commands } = await ai.plugin.commands.list();
  const mute = commands.find((each) => each.id === "host/mute").label;
  await ai.until(() => ai.end() === `done: Done · ${mute}`, "result");
  assert.ok(host.coreRuns.includes("mute"), "the player's Mute ran");
  // The player's own line for the run (the TestHost's `osd`; the player's strip), and no OSD from the plugin.
  assert.ok(host.osd.some((line) => line.startsWith("vestra.workshop · ")));
  assert.deepEqual(host.sent("osd.show"), []);
  assert.ok(ticket);
  // The table the model got: the player's commands, not Ask Vestra's own.
  const system = ai.system();
  assert.match(system, /^host\/mute \| Vestra Player: Mute \|.*\| M \| toggle off$/m);
  assert.doesNotMatch(system, /vestra\.workshop\//);
});

test("several commands: the player's line counts the steps (step/of), the result lists them", async (t) => {
  const ai = await ready(() => plan(command("vestra.video/video.rotate.90"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [row("vestra.video/video.rotate.90", "Rotate: 90°"), row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false })]);
  host.dialogAnswers.push(CONTINUE);
  const ticket = await host.runPaletteLead("rotate and mute");
  await ai.until(() => ai.end() === "done: Done · Rotate: 90°, Vestra Player: Mute", "result");
  // The first run names the rest of the plan (`then`), the later ones do not.
  assert.deepEqual(runs, [
    { id: "vestra.video/video.rotate.90", ticket, step: 1, of: 2, then: ["host/mute"] },
    { id: "host/mute", ticket, step: 2, of: 2 },
  ]);
  assert.deepEqual(runs.asks, [], "nothing in the plan asks: no sheet");
  assert.deepEqual(host.sent("osd.show"), []);
});

test("one answer covers the plan: the sheet asks once, before the first step, for every step", async (t) => {
  const ai = await ready(() =>
    plan(command("host/volume-up"), command("vestra.playlist/playlist.clear"), command("acme.obs/record.start"), command("vestra.playlist/playlist.clear"), command("host/mute")),
  );
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [
    row("host/volume-up", "Vestra Player: Volume Up"),
    row("vestra.playlist/playlist.clear", "Playlist: Clear", { dangerous: true }),
    row("acme.obs/record.start", "Mini OBS: Start Recording", { builtin: false, plugin: "Mini OBS", dangerous: true }),
    row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false }),
  ]);
  host.dialogAnswers.push(CONTINUE, CONTINUE); // consent, then Run on the sheet: no more answers queued
  const ticket = await host.runPaletteLead("louder, clear, record, clear again, mute");
  await ai.until(() => ai.end()?.startsWith("done: "), "result");
  const ids = ["host/volume-up", "vestra.playlist/playlist.clear", "acme.obs/record.start", "vestra.playlist/playlist.clear", "host/mute"];
  assert.deepEqual(runs.asks, [ids], "one sheet listing the whole plan, the repeated step too");
  assert.deepEqual(runs, [
    { id: ids[0], ticket, step: 1, of: 5, then: ids.slice(1) },
    ...ids.slice(1).map((id, i) => ({ id, ticket, step: i + 2, of: 5 })),
  ]);
  assert.equal(host.dialogAnswers.length, 0);
});

test("Don't Run on the sheet runs nothing, not even the steps that do not ask, and ends the request", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("vestra.playlist/playlist.clear"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [
    row("host/volume-up", "Vestra Player: Volume Up"),
    row("vestra.playlist/playlist.clear", "Playlist: Clear", { dangerous: true }),
    row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false }),
  ]);
  host.dialogAnswers.push(CONTINUE, DONT_RUN); // consent, then Don't Run on the sheet
  await host.runPaletteLead("louder and clear the playlist and mute");
  await ai.until(() => ai.end() === "stopped: Stopped: nothing was run", "declined");
  assert.deepEqual(runs.asks, [["host/volume-up", "vestra.playlist/playlist.clear", "host/mute"]]);
  assert.deepEqual(runs, [{ id: "host/volume-up", declined: true }], "the sheet came before the first step; nothing ran");
  await new Promise((resolve) => setTimeout(resolve, 400)); // longer than a step's gap: no later step is tried
  assert.equal(runs.length, 1);
  assert.deepEqual(ends(host), ["stopped: Stopped: nothing was run"]);
  assert.match(ai.system(), /^vestra\.playlist\/playlist\.clear \| Playlist: Clear \|.*\| asks the user first$/m);
});

test("a step the player no longer holds in the plan asks again; Don't Run then stops there, the steps before stay", async (t) => {
  // Not reachable through the plugin alone (it runs the plan it named, in
  // order, on one ticket); the player's rule is that a used-up or dropped
  // plan entry asks on its own, and the plugin then says what was not run.
  const ai = await ready(() => plan(command("host/volume-up"), command("vestra.playlist/playlist.clear"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const rows = [
    row("host/volume-up", "Vestra Player: Volume Up"),
    row("vestra.playlist/playlist.clear", "Playlist: Clear", { dangerous: true }),
    row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false }),
  ];
  // The plan is gone after the first step.
  const runs = table(host, rows, (p) => p.id === "host/volume-up" && runs.forget());
  host.dialogAnswers.push(CONTINUE, CONTINUE, DONT_RUN); // consent, Run on the sheet, Don't Run when step 2 asks again
  await host.runPaletteLead("louder and clear the playlist and mute");
  await ai.until(() => ai.end() === "stopped: Stopped: Playlist: Clear was not run", "declined");
  assert.deepEqual(runs.asks, [["host/volume-up", "vestra.playlist/playlist.clear", "host/mute"], ["vestra.playlist/playlist.clear"]]);
  assert.deepEqual(
    runs.map((run) => [run.id, !!run.declined]),
    [
      ["host/volume-up", false],
      ["vestra.playlist/playlist.clear", true],
    ],
  );
});

test("busy: the step fails with the player's message as given", async (t) => {
  const ai = await ready(() => plan(command("acme.obs/record.start")));
  t.after(() => ai.stop());
  const { host } = ai;
  const message = "another command is waiting for the user's answer";
  const runs = table(host, [row("acme.obs/record.start", "Mini OBS: Start Recording", { builtin: false, plugin: "Mini OBS", busy: true, reason: message })]);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("start recording");
  await ai.until(() => !!ai.end(), "failure");
  assert.equal(ai.end(), `failed: Step 1 failed: ${message}`);
  assert.deepEqual(runs, []);
});

test("a disabled command: the step fails with the command's reason, the steps after it do not run", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("vestra.capture/snapshot"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [
    row("host/volume-up", "Vestra Player: Volume Up"),
    row("vestra.capture/snapshot", "Snapshot: Save Snapshot", { enabled: false, reason: "Nothing to capture" }),
    row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false }),
  ]);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("louder, snapshot, mute");
  await ai.until(() => ai.end() === "failed: Step 2 failed: Nothing to capture", "failure");
  assert.deepEqual(host.sent("osd.show"), []);
  assert.deepEqual(
    runs.map((run) => run.id),
    ["host/volume-up"],
    "the step before ran, nothing after the failed one",
  );
  // The model saw the state: disabled with its reason.
  assert.match(ai.system(), /^vestra\.capture\/snapshot \| Snapshot: Save Snapshot \| - \| - \| disabled: Nothing to capture$/m);
});

test("Don't Run when the first step is the one that asks: nothing was run", async (t) => {
  const ai = await ready(() => plan(command("vestra.playlist/playlist.clear"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [
    row("vestra.playlist/playlist.clear", "Playlist: Clear", { dangerous: true }),
    row("host/mute", "Vestra Player: Mute", { checkable: true, checked: false }),
  ]);
  host.dialogAnswers.push(CONTINUE, DONT_RUN);
  await host.runPaletteLead("clear the playlist and mute");
  await ai.until(() => ai.end() === "stopped: Stopped: nothing was run", "declined");
  assert.deepEqual(runs, [{ id: "vestra.playlist/playlist.clear", declined: true }]);
  assert.deepEqual(runs.asks, [["vestra.playlist/playlist.clear", "host/mute"]]);
  assert.deepEqual(host.sent("osd.show"), []);
});

test("store fallback: the prompt runs Plugin Store… alone only when no listed command but a plugin could do it", async (t) => {
  const ai = await ready(() => plan(command("host/store")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [row("host/store", "Vestra Player: Plugin Store…", { title: "Plugin Store…" }), row("host/mute", "Vestra Player: Mute")]);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("play this YouTube link");
  await ai.until(() => ai.end() === "done: Opened Vestra Player: Plugin Store…", "store opened");
  assert.deepEqual(
    runs.map((run) => run.id),
    ["host/store"],
  );
  const system = ai.system();
  assert.ok(
    system.includes(
      "- Only when no listed command can do what the user asked but a plugin for the player could (online video sites such as YouTube or Bilibili, games and emulators, cutting and exporting clips, extra video filters or upscaling shaders), run Plugin Store… as the only step. If a listed command can do it, use that command instead.",
    ),
  );
  assert.ok(system.includes("- Otherwise, if no command does what the user asked, do not call the tool"));
  // Not the half the user left out: the model does not say which kind of plugin to look for.
  assert.doesNotMatch(system, /which (kind of )?plugin to look for/i);
});

test("a third-party command runs after the player's confirmation", async (t) => {
  const ai = await ready(() => plan(command("acme.obs/record.start")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [row("acme.obs/record.start", "Mini OBS: Start Recording", { builtin: false, plugin: "Mini OBS", dangerous: true })]);
  host.dialogAnswers.push(CONTINUE, CONTINUE); // consent, then Run
  const ticket = await host.runPaletteLead("start recording");
  await ai.until(() => ai.end() === "done: Done · Mini OBS: Start Recording", "result");
  // A plan of one command: `then` is empty, the sheet lists it alone.
  assert.deepEqual(runs, [{ id: "acme.obs/record.start", ticket, step: undefined, of: undefined, then: [] }]);
  assert.deepEqual(runs.asks, [["acme.obs/record.start"]]);
  assert.match(ai.system(), /^acme\.obs\/record\.start \| Mini OBS: Start Recording \|.*\| asks the user first, third-party: Mini OBS$/m);
});

test("an id outside the table is refused before any step runs", async (t) => {
  const ai = await ready(() => plan(command("host/mute"), command("host/format-disk")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [row("host/mute", "Vestra Player: Mute")]);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute and format the disk");
  await ai.until(() => ai.end() === "failed: Step 2 failed: There is no command “host/format-disk”", "refused");
  assert.deepEqual(runs, [], "checked before the first step ran");
});

test("a command that opens a window ends the request: the plan, and so the sheet, goes up to it", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("vestra.subtitles/tune"), command("vestra.playlist/playlist.clear")));
  t.after(() => ai.stop());
  const { host } = ai;
  const runs = table(host, [
    row("host/volume-up", "Vestra Player: Volume Up"),
    row("vestra.subtitles/tune", "Subtitles: Subtitle Tuning…"),
    row("vestra.playlist/playlist.clear", "Playlist: Clear", { dangerous: true }),
  ]);
  host.dialogAnswers.push(CONTINUE);
  const ticket = await host.runPaletteLead("louder, subtitle tuning, then clear the playlist");
  await ai.until(() => ai.end() === "done: Opened Subtitles: Subtitle Tuning…", "opened");
  assert.deepEqual(runs, [
    { id: "host/volume-up", ticket, step: 1, of: 2, then: ["vestra.subtitles/tune"] },
    { id: "vestra.subtitles/tune", ticket, step: 2, of: 2 },
  ]);
  assert.deepEqual(runs.asks, [], "the step after the window is not in the plan: no sheet for it");
});

test("personal rows never reach the model, and naming one is refused", async (t) => {
  const ai = await ready(() => plan(command("vestra.history/recent.item.0")));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  const runs = table(host, [
    row("vestra.history/recent.item.0", "Recently Played: 1  Secret Holiday Film.mkv", { path: ["Open", "Recently Played", "1  Secret Holiday Film.mkv"] }),
    row("vestra.audio/audio.track.2", "Audio Track: Director Commentary (eng)"),
    row("vestra.audio/audio.device.0", "Output Device: Anna's AirPods"),
    row("vestra.marks/chapter.3", "Chapter: The Wedding"),
    row("vestra.marks/bookmark.0", "Bookmark: Grandma's speech"),
    row("vestra.subtitles/track-1", "Subtitle Track: Holiday.en.srt"),
    row("vestra.subtitles/second-position-top", "Secondary Subtitles: Top"),
    row("acme.clips/open.4", "Clips: holiday-2024.mp4", { builtin: false, plugin: "Clips" }),
    row("acme.clips/folder", "Clips: C:\\Users\\anna\\Videos", { builtin: false, plugin: "Clips" }),
    row("acme.clips/record", "Clips: Record Clip", { builtin: false, plugin: "Clips", dangerous: true }),
    row("host/mute", "Vestra Player: Mute"),
  ]);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("open the film from last summer");
  await ai.until(() => ai.end()?.startsWith("failed: Step 1 failed"), "refused");
  assert.equal(ai.end(), "failed: Step 1 failed: There is no command “vestra.history/recent.item.0”");
  assert.deepEqual(runs, []);
  const sent = JSON.stringify(mock.chats[0].body);
  for (const secret of ["Holiday", "holiday", "Commentary", "AirPods", "Wedding", "Grandma", "anna", "recent.item", "audio.track", "audio.device", "chapter.3", "bookmark.0", "track-1"])
    assert.ok(!sent.includes(secret), `the request names ${secret}`);
  // Commands of the same plugins that are not built from data still go.
  for (const kept of ["vestra.subtitles/second-position-top", "acme.clips/record", "host/mute"]) assert.ok(sent.includes(kept), kept);
});

test("consent given for the first slice (scope v1) is asked again now that the command list is sent", async (t) => {
  const data = await dataDirectory();
  const mock = await server(() => plan(command("host/mute")));
  // What 2.2.38's first slice saved after Continue.
  await fs.writeFile(path.join(data, "askvestra.json"), JSON.stringify({ providers: [`v1 custom-openai ${mock.url}`] }));
  const ai = await startAi({ compat: mock.url, builtin: true, data });
  t.after(() => ai.stop());
  const { host } = ai;
  host.openElsewhere("/media/a.mkv");
  await host.settle();
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => host.coreRuns.includes("mute") && !host.paletteLead.status, "result");
  assert.equal(host.confirms.length, 1, "asked again");
  assert.match(host.confirmTexts[0].detail, /command list/);
  await new Promise((resolve) => setTimeout(resolve, 500)); // the store writes after its debounce
  const saved = JSON.parse(await fs.readFile(path.join(data, "askvestra.json"), "utf8"));
  assert.ok(saved.providers.includes(`v2 custom-openai ${mock.url}`), JSON.stringify(saved));
});
