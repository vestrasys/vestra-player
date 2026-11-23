// Ask Vestra, the palette row and the request's life (PROPOSAL §9 steps 4–5):
// the lead row, Enter → consent once → the model's plan (a local mock model,
// never a paid one) → the player's command through commands.run → the
// palette status the player shows in its status strip (no OSD of its own);
// Stop, a newer request replacing the running one, a model error, no model
// set up, and a plan the plugin refuses. Commands only (user ruling
// 2026-11-11): no speed step, no Undo.
import assert from "node:assert/strict";
import os from "node:os";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { RUNS_PER_SECOND, STEP_GAP_MS, pace } from "../askvestra/ask.ts";
import { KEY, server, startAi } from "./fixture.mjs";
import { guard, shown, track } from "./lead-status.mjs";

guard(test);

const plan = (...steps) => ({ toolCalls: [{ name: "run_steps", args: { steps } }] });
const command = (id) => ({ kind: "command", id });
const MUTE = command("host/mute");
const CONTINUE = { confirmed: true, checked: false };
const CANCEL = { confirmed: false, checked: false };
const leadText = (host, field) => shown(host, host.paletteLead?.[field]);
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
/** Replies by model call (the model list requests do not count): `replies[i]` for call i, the last one after. */
const byCall = (...replies) => {
  let calls = 0;
  return () => replies[Math.min(calls++, replies.length - 1)];
};

/** The plugin as a built-in, a mock model set up as "Custom · OpenAI format", a file playing. */
async function ready(reply, options = {}) {
  const mock = await server(reply);
  const ai = await startAi({ compat: mock.url, builtin: true, ...options });
  track(ai.host);
  ai.host.openElsewhere("/media/Secret Holiday Film.mkv");
  ai.host.setSource({ duration: 600, position: 30, paused: false });
  await ai.host.settle();
  // The palette's label of the player's Mute, as the result line names it.
  const { commands } = await ai.plugin.commands.list();
  const mute = commands.find((row) => row.id === "host/mute").label;
  return { ...ai, mock, mute, end: () => ends(ai.host).at(-1), ends: () => ends(ai.host) };
}

test("lead row: title, icon, placeholder and hint; 'Set up a model first' until a model is set up", async (t) => {
  const mock = await server(() => plan(MUTE));
  const ai = await startAi({ builtin: true });
  t.after(() => ai.stop());
  const { host } = ai;
  await ai.until(() => host.paletteLead !== null, "lead row");
  assert.equal(leadText(host, "title"), "Ask Vestra");
  assert.equal(host.paletteLead.icon, "ai");
  assert.equal(leadText(host, "placeholder"), "Type a command or ask Vestra");
  assert.equal(leadText(host, "empty"), "Type a request or a question");
  assert.equal(leadText(host, "note"), "Set up a model first");
  assert.equal(host.paletteLead.status ?? null, null);
  await ai.configure(mock.url);
  await ai.until(() => !host.paletteLead.note, "note gone");
});

test("lead row: shown in the interface language", async (t) => {
  const ai = await startAi({ builtin: true, locale: "zh-Hans" });
  t.after(() => ai.stop());
  await ai.until(() => ai.host.paletteLead !== null, "lead row");
  assert.equal(leadText(ai.host, "title"), "问 Vestra");
  assert.equal(leadText(ai.host, "note"), "请先设置模型");
});

test("not built in (a development load): the row is refused, the rest of the plugin works", async (t) => {
  const host = new TestHost();
  host.expectError("builtin-only"); // the refusal of the lead row this test is about
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
  t.after(() => started.stop());
  await host.settle();
  assert.equal(host.paletteLead, null);
  await host.command("workshop.open").run();
  assert.equal(host.window("ai").visible, true);
});

test("no model set up: Enter opens the settings page, nothing is sent", async (t) => {
  const mock = await server(() => plan(MUTE));
  const ai = await startAi({ builtin: true });
  t.after(() => ai.stop());
  await ai.host.runPaletteLead("mute");
  await ai.until(() => ai.host.settings.shows.includes("byok"), "settings shown");
  assert.equal(mock.chats.length, 0);
  assert.equal(ai.host.confirms.length, 0);
});

test("an empty query opens the AI window", async (t) => {
  const ai = await startAi({ builtin: true });
  t.after(() => ai.stop());
  await ai.host.window("ai").userClose();
  await ai.host.settle();
  assert.ok(!ai.host.windows.has("ai") || !ai.host.window("ai").visible, "closed");
  await ai.host.runPaletteLead("");
  await ai.until(() => ai.host.windows.has("ai") && ai.host.window("ai").visible, "window shown");
});

test("one sentence → consent once → plan → the command runs → a plain result line", async (t) => {
  const ai = await ready(() => plan(MUTE));
  t.after(() => ai.stop());
  const { host, mock } = ai;

  // Cancel at the consent dialog: nothing is sent, nothing is remembered.
  host.dialogAnswers.push(CANCEL);
  await host.runPaletteLead("mute");
  await ai.until(() => host.confirms.length === 1, "consent asked");
  assert.equal(host.confirmTexts[0].message, "Ask Vestra sends your request to Custom · OpenAI format");
  assert.match(host.confirmTexts[0].detail, /the player.s command list .* and whether something is open and playing go to this provider/);
  assert.match(host.confirmTexts[0].detail, /File names, playlists, history, track and chapter titles and subtitles are not sent/);
  assert.equal(host.confirmTexts[0].confirm, "Continue");
  await host.settle();
  assert.equal(mock.chats.length, 0);

  host.dialogAnswers.push(CONTINUE);
  const ticket = await host.runPaletteLead("mute");
  await ai.until(() => ai.end() === `done: Done · ${ai.mute}`, "result");
  assert.ok(host.coreRuns.includes("mute"));
  assert.equal(host.confirms.length, 2);
  // The end state, then the row without a status, then the ticket's done: no timer of the plugin's own.
  await ai.until(() => host.sent("palette.lead.done").some((p) => p.ticket === ticket), "done");
  const order = host.calls.map((call) =>
    call.method === "palette.lead.done" && call.params.ticket === ticket
      ? "done"
      : call.method === "palette.lead.set" && call.params.status?.state === "done"
        ? "end"
        : call.method === "palette.lead.set" && !call.params.status
          ? "idle"
          : null,
  );
  const end = order.lastIndexOf("end");
  assert.ok(end >= 0 && order.indexOf("idle", end) > end && order.indexOf("done", end) > order.indexOf("idle", end), order.filter(Boolean).join(" "));
  // Planning… while the model planned (working: the default state), then one end state.
  const statuses = host.sent("palette.lead.set").map((p) => p.status);
  assert.ok(statuses.some((status) => status && shown(host, status.text) === "Vestra is working · Planning…" && status.state === undefined), "Planning…");
  assert.deepEqual(ai.ends(), [`done: Done · ${ai.mute}`], "one end state");
  // Nothing on the OSD from the plugin: no progress, result or Stop button there.
  assert.deepEqual(host.sent("osd.show"), []);
  // The lead row is back to its idle state, and the end state is not kept on it.
  await ai.until(() => !host.paletteLead.status, "status cleared");

  // What the model saw: the request and whether something plays, never the file.
  assert.equal(mock.chats.length, 1);
  const sent = JSON.stringify(mock.chats[0].body);
  assert.match(sent, /Request: mute/);
  assert.match(sent, /Player state: \{\\"open\\":true,\\"playing\\":true\}/);
  assert.doesNotMatch(sent, /Secret Holiday Film|\/media\/|"rate"|position|duration/);
  assert.doesNotMatch(sent, new RegExp(KEY));
  // No path of this machine either: pi's <cwd> section is a neutral name, not the data directory under the user's home.
  // JSON doubles a backslash: both spellings of a Windows path become forward slashes.
  const decoded = JSON.stringify(mock.chats[0].body.messages).replace(/\\\\/g, "/").replace(/\\/g, "/").toLowerCase();
  for (const dir of [ai.plugin.dataDirectory, os.homedir(), os.tmpdir()]) {
    const plain = dir.replace(/\\/g, "/").toLowerCase();
    assert.ok(!decoded.includes(plain), `the request names ${dir}`);
  }
  assert.deepEqual(
    mock.chats[0].body.tools.map((tool) => tool.function.name),
    ["run_steps"],
  );
  // No step kind but commands in the prompt.
  assert.doesNotMatch(mock.chats[0].body.messages[0].content, /"kind": "speed"/);

  // Consent is not asked again for the same provider.
  await host.runPaletteLead("mute again");
  await ai.until(() => mock.chats.length === 2 && host.coreRuns.filter((run) => run === "mute").length === 2, "second result");
  assert.equal(host.confirms.length, 2);
});

test("Stop while planning: Enter on the status row stops the request; the plugin is kept running meanwhile", async (t) => {
  const ai = await ready(() => ({ hang: true }));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => mock.chats.length === 1 && !!host.paletteLead.status, "planning");
  assert.equal(shown(host, host.paletteLead.status.text), "Vestra is working · Planning…");
  assert.equal(host.paletteLead.status.state, undefined, "working");
  assert.equal(shown(host, host.paletteLead.status.note), "Enter stops");
  // A busy command meanwhile keeps the plugin out of standby; Stop itself stays runnable.
  assert.equal(host.command("askvestra.working").state.busy, true);
  assert.equal(host.command("askvestra.stop").state.enabled, true);
  assert.equal(host.command("askvestra.stop").state.busy, false);
  await host.runPaletteLead("", { status: true });
  await ai.until(() => ai.end() === "stopped: Stopped" && !host.paletteLead.status, "stopped");
  assert.ok(!host.coreRuns.includes("mute"));
  assert.equal(host.command("askvestra.working").state.busy, false);
  assert.equal(host.command("askvestra.stop").state.enabled, false);
});

test("the busy helper command is in no menu, palette or bar, and has no key", async (t) => {
  const ai = await startAi({ builtin: true });
  t.after(() => ai.stop());
  const { declaration } = ai.host.command("askvestra.working");
  assert.equal(declaration.focus, "panel");
  assert.ok(!declaration.path?.length);
  assert.ok(!declaration.bindings?.length);
  const { commands } = await ai.plugin.commands.list();
  assert.ok(!commands.some((row) => row.id.endsWith("/askvestra.working")), "not in the palette");
  assert.ok(!ai.host.commands.has("askvestra.undo"), "no Undo command");
});

test("Stop from the palette command Vestra: Stop", async (t) => {
  const ai = await ready(() => ({ hang: true }));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => mock.chats.length === 1 && !!host.paletteLead.status, "planning");
  await host.command("askvestra.stop").run();
  await ai.until(() => ai.end() === "stopped: Stopped", "stopped");
  assert.ok(!host.coreRuns.includes("mute"));
});

test("a new request stops the running one and runs", async (t) => {
  const ai = await ready(byCall({ hang: true }, plan(command("host/volume-up"))));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("first");
  await ai.until(() => mock.chats.length === 1, "first planning");
  await host.runPaletteLead("louder");
  await ai.until(() => host.coreRuns.includes("volume-up") && !host.paletteLead.status, "second result");
  assert.equal(ai.ends().length, 1, "a replaced request is not reported as stopped");
  assert.match(ai.end(), /^done: Done · /);
  assert.equal(mock.chats.length, 2);
});

test("a model error: a note names the provider and the message", async (t) => {
  const ai = await ready(() => ({ status: 401, error: "Incorrect API key provided" }));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => ai.end()?.startsWith("note: Could not reach"), "error");
  // The provider message is cut to the player's 80 characters (it was refused whole: the user saw nothing).
  assert.match(ai.end(), /^note: Could not reach Custom · OpenAI format: \S/);
  const [text] = ai.ends().map((line) => line.slice("note: ".length));
  assert.ok([...text].length <= 160, text);
  assert.ok(text.includes("Incorrect API key provided") || text.endsWith("…"), text);
  assert.deepEqual(host.sent("osd.show"), []);
  assert.ok(!host.coreRuns.includes("mute"));
  await ai.until(() => !host.paletteLead.status, "status cleared");
});

test("the model answers without steps: its sentence is shown", async (t) => {
  const ai = await ready(() => ({ text: ["No command changes the picture's warmth."] }));
  t.after(() => ai.stop());
  ai.host.dialogAnswers.push(CONTINUE);
  await ai.host.runPaletteLead("warmer");
  await ai.until(() => ai.end() === "note: No command changes the picture's warmth.", "refusal");
});

test("a long sentence of the model is cut to the player's 80 characters, in the interface language too", async (t) => {
  for (const locale of ["en", "zh-Hans"]) {
    const sentence = `${"没有命令可以把音量设为某个具体数值。".repeat(4)} ${"No command sets an exact volume. ".repeat(4)}`;
    const ai = await ready(() => ({ text: [sentence] }), { locale });
    try {
      ai.host.dialogAnswers.push(CONTINUE);
      await ai.host.runPaletteLead("volume 37");
      await ai.until(() => !!ai.end(), `${locale}: sentence`);
      const text = ai.end().slice("note: ".length);
      assert.equal([...text].length, 160, text);
      assert.ok(text.endsWith("…"));
      assert.ok(sentence.startsWith(text.slice(0, -1)));
      // Accepted by the TestHost as well: nothing was logged as refused (guard), and the row is idle again.
      await ai.until(() => !ai.host.paletteLead.status, `${locale}: idle`);
    } finally {
      await ai.stop();
    }
  }
});

test("a plan the plugin refuses runs no step: another step kind (the old speed step), too many steps", async (t) => {
  const many = Array.from({ length: 9 }, () => MUTE);
  const ai = await ready(byCall(plan(MUTE, { kind: "speed", rate: 1.5 }), plan(...many)));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute and faster");
  await ai.until(() => ai.end() === "failed: Step 2 failed: Vestra cannot do “speed”", "unknown kind");
  assert.equal(host.source.rate, 1);
  assert.ok(!host.coreRuns.includes("mute"), "checked before any step runs");
  await host.runPaletteLead("mute nine times");
  await ai.until(() => ai.end() === "note: The plan has more than 8 steps", "too many");
  assert.ok(!host.coreRuns.includes("mute"));
});

/** Records when each of the player's own commands ran (`host.coreRuns`), in `performance.now()` time. */
const timed = (host) => {
  const times = [];
  const push = host.coreRuns.push.bind(host.coreRuns);
  host.coreRuns.push = (...runs) => {
    for (let i = 0; i < runs.length; i++) times.push(performance.now());
    return push(...runs);
  };
  return times;
};
/** The gaps between successive times, in ms. */
const gaps = (times) => times.slice(1).map((at, i) => at - times[i]);

test("a plan of the most steps runs them all within the player's rate (4 a second per ticket), each step at least STEP_GAP_MS after the one before", async (t) => {
  const steps = Array.from({ length: 8 }, (_, i) => command(i % 2 ? "host/volume-down" : "host/volume-up"));
  const ai = await ready(() => plan(...steps));
  t.after(() => ai.stop());
  const { host } = ai;
  const times = timed(host);
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("up and down four times");
  await ai.until(() => !host.paletteLead.status && !!ai.end(), "result", 30_000);
  // Eight labels do not fit the player's 80 characters: the list ends with a cut label.
  assert.match(ai.end(), /^done: Done · Vestra Player: .*…$/);
  assert.ok([...ai.end().slice("done: ".length)].length <= 160, ai.end());
  const ran = host.coreRuns.filter((run) => run === "volume-up" || run === "volume-down");
  assert.equal(ran.length, 8);
  // The player's own lines, one per step, in order (the TestHost records them in `osd`; the player draws them in the strip).
  const lines = host.osd.filter((line) => / · \d\/8 · /.test(line));
  assert.deepEqual(
    lines.map((line) => line.match(/ · (\d)\/8 · /)[1]),
    ["1", "2", "3", "4", "5", "6", "7", "8"],
  );
  // The first run named the other seven (`then`), the later ones nothing; no sheet (nothing asks).
  const sent = host.sent("commands.run");
  assert.deepEqual(
    sent[0].then,
    steps.slice(1).map((step) => step.id),
  );
  assert.ok(sent.slice(1).every((p) => p.then === undefined));
  assert.equal(host.dialogAnswers.length, 0);
  // Each step's line stays at least STEP_GAP_MS (measured here from run to run, which is longer than from the answer).
  for (const gap of gaps(times)) assert.ok(gap >= STEP_GAP_MS - 1, `steps ${gaps(times).map(Math.round).join(", ")} ms apart`);
});

for (const length of [5, 6, 7, 8]) {
  test(`a plan of ${length} steps with one that asks: one answer, every step runs, within the player's limits (the TestHost's plan model)`, async (t) => {
    // The player's Close (`host/stop`) asks first; the TestHost (SDK 1.7.16)
    // judges the plan as the player does and applies its rates.
    const steps = [...Array.from({ length: length - 1 }, () => command("host/volume-up")), command("host/stop")];
    const ai = await ready(() => plan(...steps));
    t.after(() => ai.stop());
    const { host } = ai;
    host.dialogAnswers.push(CONTINUE, CONTINUE); // consent, Run on the sheet
    await host.runPaletteLead(`louder ${length - 1} times and close`);
    await ai.until(() => !host.paletteLead.status && !!ai.end(), "result", 30_000);
    assert.match(ai.end(), /^done: /);
    assert.equal(host.coreRuns.filter((run) => run === "volume-up").length, length - 1);
    assert.ok(host.coreRuns.includes("stop"), "Close ran without asking again");
    assert.equal(host.dialogAnswers.length, 0, "one answer covered the plan");
  });
}

test("Don't Run on the sheet (the TestHost's plan model): nothing runs, the request ends", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("host/stop"), MUTE));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE, CANCEL); // consent, Don't Run on the sheet
  await host.runPaletteLead("louder, close, mute");
  await ai.until(() => ai.end() === "stopped: Stopped: nothing was run", "declined");
  await new Promise((resolve) => setTimeout(resolve, 2 * STEP_GAP_MS));
  assert.deepEqual(
    host.coreRuns.filter((run) => ["volume-up", "stop", "mute"].includes(run)),
    [],
  );
  assert.equal(host.sent("commands.run").length, 1, "no later step was tried");
});

test("the row says Vestra is working from the moment it is sent; Enter on it then stops, nothing is sent again", async (t) => {
  // Shift+Enter keeps the palette open on the request; the player shows the
  // row's status for it (host command_palette::lead_shows_status) as soon as
  // the plugin sets one, so the plugin sets it before any wait (here the consent).
  const ai = await ready(() => plan(MUTE));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  let answer;
  const answered = new Promise((resolve) => (answer = resolve));
  host.handle("dialogs.confirm", async () => {
    await answered;
    return CONTINUE;
  });
  void host.runPaletteLead("mute", { keep: true });
  await ai.until(() => !!host.paletteLead.status, "working");
  assert.equal(shown(host, host.paletteLead.status.text), "Vestra is working · Planning…");
  assert.equal(host.command("askvestra.working").state.busy, true);
  assert.equal(host.command("askvestra.stop").state.enabled, true);
  // The second Enter reaches the plugin as the status row's (not awaited:
  // the test host's settle would wait for the consent dialog held open).
  void host.runPaletteLead("mute", { keep: true, status: true });
  await ai.until(() => !host.paletteLead.status, "status cleared");
  answer();
  await host.settle();
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.equal(mock.chats.length, 0, "nothing sent");
  assert.deepEqual(ai.ends(), ["stopped: Stopped"], "the strip says Stopped once");
  assert.ok(!host.coreRuns.includes("mute"));
  assert.equal(host.command("askvestra.stop").state.enabled, false);
});

test("pace: the first step at once, a later one STEP_GAP_MS after the answer before it, and one second after the answer RUNS_PER_SECOND before, never less", async () => {
  const signal = new AbortController().signal;
  assert.ok(STEP_GAP_MS >= 1000 / RUNS_PER_SECOND, "the gap alone keeps the rate");
  let start = performance.now();
  await pace([], signal);
  assert.ok(performance.now() - start < 50, "the first step: no wait");
  start = performance.now();
  await pace([start - 2000, start - 1500, start], signal);
  assert.ok(performance.now() >= start + STEP_GAP_MS, `waited ${performance.now() - start} ms after the last answer`);
  // Four answers at once (as if the gap had not been kept): the rate decides.
  const answered = Array.from({ length: RUNS_PER_SECOND }, () => performance.now());
  await pace(answered, signal);
  assert.ok(performance.now() >= answered[0] + 1000, `waited ${performance.now() - answered[0]} ms`);
});

test("the consent dialog refused by the player (busy): the row and the busy command do not stay working", async (t) => {
  const ai = await ready(() => plan(MUTE));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.handle("dialogs.confirm", async () => {
    const error = new Error("a native dialog is already open");
    error.code = "busy";
    throw error;
  });
  await host.runPaletteLead("mute");
  await ai.until(() => (host.paletteLead.status ?? null) === null, "status cleared");
  assert.equal(host.command("askvestra.working").state.busy, false);
  assert.equal(host.command("askvestra.stop").state.enabled, false);
  assert.equal(mock.chats.length, 0);
});

test("Stop between steps: the paced wait ends at once", async (t) => {
  const steps = Array.from({ length: 8 }, () => command("host/volume-up"));
  const ai = await ready(() => plan(...steps));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("louder eight times");
  // Each step waits STEP_GAP_MS after the answer to the one before.
  await ai.until(() => host.coreRuns.filter((run) => run === "volume-up").length === 2, "two steps", 30_000);
  const started = performance.now();
  await host.command("askvestra.stop").run();
  await ai.until(() => ai.end() === "stopped: Stopped", "stopped");
  assert.ok(performance.now() - started < 900, "at once");
  const ranAtStop = host.coreRuns.filter((run) => run === "volume-up").length;
  assert.ok(ranAtStop <= 3, `${ranAtStop} steps ran`);
  await new Promise((resolve) => setTimeout(resolve, 3 * STEP_GAP_MS));
  assert.equal(host.coreRuns.filter((run) => run === "volume-up").length, ranAtStop, "no step after Stop");
});

test("consent: two requests while the dialog is open ask once and the latest one runs; a new Base URL asks again", async (t) => {
  const ai = await ready(() => plan(MUTE));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  let answer;
  const answered = new Promise((resolve) => (answer = resolve));
  let asked = 0;
  host.handle("dialogs.confirm", async () => {
    asked++;
    await answered;
    return CONTINUE;
  });
  void host.runPaletteLead("first");
  await ai.until(() => asked === 1, "dialog open");
  void host.runPaletteLead("second");
  await new Promise((resolve) => setTimeout(resolve, 200));
  answer();
  await ai.until(() => ai.end() === `done: Done · ${ai.mute}`, "result");
  assert.equal(asked, 1);
  assert.equal(mock.chats.length, 1);
  assert.match(JSON.stringify(mock.chats[0].body.messages), /Request: second/);

  // Another Base URL is another recipient.
  const other = await server(() => plan(command("host/volume-up")));
  ai.page.emit("set-base-url", other.url);
  await host.settle();
  await host.runPaletteLead("louder");
  await ai.until(() => asked === 2, "asked again");
  await ai.until(() => host.coreRuns.includes("volume-up"), "second result");
  assert.equal(other.chats.length, 1);
});
