// Ask Vestra, the vertical slice (PROPOSAL §9 step 4): the palette's lead
// row, Enter → consent once → the model's plan (a local mock model, never a
// paid one) → "Speed 1.25×" with the plugin's own setRate → the OSD with
// Undo → Undo; Stop, a newer request replacing the running one, a model
// error, no model set up, and a plan the plugin refuses.
import assert from "node:assert/strict";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { KEY, server, startAi } from "./fixture.mjs";

const plan = (...steps) => ({ toolCalls: [{ name: "run_steps", args: { steps } }] });
const speed = (rate) => ({ kind: "speed", rate });
const CONTINUE = { confirmed: true, checked: false };
const CANCEL = { confirmed: false, checked: false };
/** A text of the lead row (wire form: a string or { $vestraText, args }) as the palette shows it. */
const shown = (host, value) => {
  if (value === undefined || value === null) return value;
  return typeof value === "string" ? host.translate(value) : host.translate(value.$vestraText, value.args ?? []);
};
const leadText = (host, field) => shown(host, host.paletteLead?.[field]);
/** Replies by model call (the model list requests do not count): `replies[i]` for call i, the last one after. */
const byCall = (...replies) => {
  let calls = 0;
  return () => replies[Math.min(calls++, replies.length - 1)];
};

/** The plugin as a built-in, a mock model set up as "Custom · OpenAI format", a file playing at 1×. */
async function ready(reply, options = {}) {
  const mock = await server(reply);
  const ai = await startAi({ compat: mock.url, builtin: true, ...options });
  ai.host.openElsewhere("/media/Secret Holiday Film.mkv");
  ai.host.setSource({ duration: 600, position: 30, paused: false });
  await ai.host.settle();
  return { ...ai, mock, osd: () => ai.host.osd.at(-1) };
}

test("lead row: title, icon, placeholder and hint; 'Set up a model first' until a model is set up", async (t) => {
  const mock = await server(() => plan(speed(1.25)));
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
  const started = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
  t.after(() => started.stop());
  await host.settle();
  assert.equal(host.paletteLead, null);
  await host.command("workshop.open").run();
  assert.equal(host.window("ai").visible, true);
});

test("no model set up: Enter opens the settings page, nothing is sent", async (t) => {
  const mock = await server(() => plan(speed(1.25)));
  const ai = await startAi({ builtin: true });
  t.after(() => ai.stop());
  await ai.host.runPaletteLead("1.25x speed");
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

test("one sentence → consent once → plan → setRate → OSD with Undo → Undo restores the speed", async (t) => {
  const ai = await ready(() => plan(speed(1.25)));
  t.after(() => ai.stop());
  const { host, mock } = ai;

  // Cancel at the consent dialog: nothing is sent, nothing is remembered.
  host.dialogAnswers.push(CANCEL);
  await host.runPaletteLead("1.25x speed");
  await ai.until(() => host.confirms.length === 1, "consent asked");
  assert.equal(host.confirmTexts[0].message, "Ask Vestra sends your request to Custom · OpenAI format");
  assert.match(host.confirmTexts[0].detail, /File names, playlists, history and subtitles are not sent/);
  assert.equal(host.confirmTexts[0].confirm, "Continue");
  await host.settle();
  assert.equal(mock.chats.length, 0);

  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("1.25x speed");
  await ai.until(() => ai.osd() === "Vestra · Speed 1.25×", "result OSD");
  assert.equal(host.source.rate, 1.25);
  assert.equal(host.confirms.length, 2);
  assert.ok(host.osd.includes("Vestra · Planning…"));
  assert.deepEqual(host.osdActions.at(-1), [{ title: "Undo", command: "askvestra.undo" }]);
  assert.deepEqual(host.osdActions[host.osd.lastIndexOf("Vestra · Planning…")], [{ title: "Stop", command: "askvestra.stop" }]);
  // The lead row is back to its idle state.
  await ai.until(() => !host.paletteLead.status, "status cleared");

  // What the model saw: the request and the playback state, never the file.
  assert.equal(mock.chats.length, 1);
  const sent = JSON.stringify(mock.chats[0].body);
  assert.match(sent, /1\.25x speed/);
  assert.match(sent, /\\"rate\\":1/);
  assert.doesNotMatch(sent, /Secret Holiday Film|\/media\//);
  assert.doesNotMatch(sent, new RegExp(KEY));
  assert.deepEqual(
    mock.chats[0].body.tools.map((tool) => tool.function.name),
    ["run_steps"],
  );

  // Undo, from the OSD's button.
  await host.pressOsdAction(0);
  await ai.until(() => ai.osd() === "Vestra · Undone", "undone");
  assert.equal(host.source.rate, 1);
  // Nothing more to undo.
  await host.command("askvestra.undo").run();
  await ai.until(() => ai.osd() === "Vestra · Nothing to undo", "nothing to undo");

  // Consent is not asked again for the same provider.
  await host.runPaletteLead("2x speed");
  await ai.until(() => mock.chats.length === 2 && ai.osd() === "Vestra · Speed 1.25×", "second result");
  assert.equal(host.confirms.length, 2);
});

test("several steps: the OSD counts them, Undo restores the speed from before the request", async (t) => {
  const ai = await ready(() => plan(speed(1.5), speed(2)));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("1.5 then double");
  await ai.until(() => ai.osd() === "Vestra · Speed 1.5×, Speed 2×", "result");
  assert.ok(host.osd.includes("Vestra · 1/2 · Speed 1.5×"));
  assert.ok(host.osd.includes("Vestra · 2/2 · Speed 2×"));
  assert.equal(host.source.rate, 2);
  await host.command("askvestra.undo").run();
  await ai.until(() => ai.osd() === "Vestra · Undone", "undone");
  assert.equal(host.source.rate, 1);
});

test("Stop while planning: Enter on the status row stops the request, the speed stays", async (t) => {
  const ai = await ready(() => ({ hang: true }));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("1.25x speed");
  await ai.until(() => mock.chats.length === 1 && !!host.paletteLead.status, "planning");
  assert.equal(shown(host, host.paletteLead.status.text), "Vestra is working · Planning…");
  assert.equal(shown(host, host.paletteLead.status.note), "Enter stops");
  // The undo command is busy meanwhile (the player keeps the plugin running).
  assert.equal(host.command("askvestra.undo").state.busy, true);
  assert.equal(host.command("askvestra.stop").state.enabled, true);
  await host.runPaletteLead("", { status: true });
  await ai.until(() => ai.osd() === "Vestra · Stopped" && !host.paletteLead.status, "stopped");
  assert.equal(host.source.rate, 1);
  assert.equal(host.command("askvestra.undo").state.busy, false);
  assert.equal(host.command("askvestra.stop").state.enabled, false);
});

test("Stop from the OSD's button", async (t) => {
  const ai = await ready(() => ({ hang: true }));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("1.25x speed");
  await ai.until(() => mock.chats.length === 1 && ai.osd() === "Vestra · Planning…", "planning");
  await host.pressOsdAction(0);
  await ai.until(() => ai.osd() === "Vestra · Stopped", "stopped");
  assert.equal(host.source.rate, 1);
});

test("a new request stops the running one and runs", async (t) => {
  const ai = await ready(byCall({ hang: true }, plan(speed(0.75))));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("first");
  await ai.until(() => mock.chats.length === 1, "first planning");
  await host.runPaletteLead("slower");
  await ai.until(() => ai.osd() === "Vestra · Speed 0.75×", "second result");
  assert.equal(host.source.rate, 0.75);
  assert.ok(!host.osd.includes("Vestra · Stopped"), "a replaced request is not reported as stopped");
  assert.equal(mock.chats.length, 2);
});

test("a model error: the OSD names the provider and the message", async (t) => {
  const ai = await ready(() => ({ status: 401, error: "Incorrect API key provided" }));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("1.25x speed");
  await ai.until(() => ai.osd()?.startsWith("Vestra · Could not reach"), "error OSD");
  assert.match(ai.osd(), /^Vestra · Could not reach Custom · OpenAI format: .*Incorrect API key provided/);
  assert.deepEqual(host.osdActions.at(-1), []);
  assert.equal(host.source.rate, 1);
  await ai.until(() => !host.paletteLead.status, "status cleared");
});

test("the model answers without steps: its sentence is shown", async (t) => {
  const ai = await ready(() => ({ text: ["I cannot change the picture yet."] }));
  t.after(() => ai.stop());
  ai.host.dialogAnswers.push(CONTINUE);
  await ai.host.runPaletteLead("warmer");
  await ai.until(() => ai.osd() === "Vestra · I cannot change the picture yet.", "refusal");
});

test("a plan the plugin refuses runs no step: unknown kind, speed out of range", async (t) => {
  const ai = await ready(byCall(plan(speed(1.5), { kind: "warmth", amount: 20 }), plan(speed(9))));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("faster and warmer");
  await ai.until(() => ai.osd() === "Vestra · Step 2 failed: Vestra cannot do “warmth” yet", "unknown kind");
  assert.equal(host.source.rate, 1, "checked before any step runs");
  await host.runPaletteLead("9x");
  await ai.until(() => ai.osd() === "Vestra · Step 1 failed: The speed must be between 0.25× and 4×", "out of range");
  assert.equal(host.source.rate, 1);
});

test("nothing playing: the step fails with the reason", async (t) => {
  const mock = await server(() => plan(speed(1.25)));
  const ai = await startAi({ compat: mock.url, builtin: true });
  t.after(() => ai.stop());
  ai.host.dialogAnswers.push(CONTINUE);
  await ai.host.runPaletteLead("1.25x speed");
  await ai.until(() => ai.host.osd.at(-1) === "Vestra · Step 1 failed: Nothing is playing", "failure");
});
