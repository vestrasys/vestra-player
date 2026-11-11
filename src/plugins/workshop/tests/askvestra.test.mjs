// Ask Vestra, the palette row and the request's life (PROPOSAL §9 steps 4–5):
// the lead row, Enter → consent once → the model's plan (a local mock model,
// never a paid one) → the player's command through commands.run → the OSD;
// Stop, a newer request replacing the running one, a model error, no model
// set up, and a plan the plugin refuses. Commands only (user ruling
// 2026-11-11): no speed step, no Undo.
import assert from "node:assert/strict";
import os from "node:os";
import test from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { KEY, server, startAi } from "./fixture.mjs";

const plan = (...steps) => ({ toolCalls: [{ name: "run_steps", args: { steps } }] });
const command = (id) => ({ kind: "command", id });
const MUTE = command("host/mute");
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

/** The plugin as a built-in, a mock model set up as "Custom · OpenAI format", a file playing. */
async function ready(reply, options = {}) {
  const mock = await server(reply);
  const ai = await startAi({ compat: mock.url, builtin: true, ...options });
  ai.host.openElsewhere("/media/Secret Holiday Film.mkv");
  ai.host.setSource({ duration: 600, position: 30, paused: false });
  await ai.host.settle();
  // The palette's label of the player's Mute, as the result line names it.
  const { commands } = await ai.plugin.commands.list();
  const mute = commands.find((row) => row.id === "host/mute").label;
  return { ...ai, mock, mute, osd: () => ai.host.osd.at(-1) };
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
  await host.runPaletteLead("mute");
  await ai.until(() => ai.osd() === `Vestra · ${ai.mute}`, "result OSD");
  assert.ok(host.coreRuns.includes("mute"));
  assert.equal(host.confirms.length, 2);
  assert.ok(host.osd.includes("Vestra · Planning…"));
  assert.deepEqual(host.osdActions.at(-1), [], "no Undo");
  assert.deepEqual(host.osdActions[host.osd.lastIndexOf("Vestra · Planning…")], [{ title: "Stop", command: "askvestra.stop" }]);
  // The lead row is back to its idle state.
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
  assert.equal(shown(host, host.paletteLead.status.note), "Enter stops");
  // A busy command meanwhile keeps the plugin out of standby; Stop itself stays runnable.
  assert.equal(host.command("askvestra.working").state.busy, true);
  assert.equal(host.command("askvestra.stop").state.enabled, true);
  assert.equal(host.command("askvestra.stop").state.busy, false);
  await host.runPaletteLead("", { status: true });
  await ai.until(() => ai.osd() === "Vestra · Stopped" && !host.paletteLead.status, "stopped");
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

test("Stop from the OSD's button", async (t) => {
  const ai = await ready(() => ({ hang: true }));
  t.after(() => ai.stop());
  const { host, mock } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => mock.chats.length === 1 && ai.osd() === "Vestra · Planning…", "planning");
  await host.pressOsdAction(0);
  await ai.until(() => ai.osd() === "Vestra · Stopped", "stopped");
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
  assert.ok(!host.osd.includes("Vestra · Stopped"), "a replaced request is not reported as stopped");
  assert.equal(mock.chats.length, 2);
});

test("a model error: the OSD names the provider and the message", async (t) => {
  const ai = await ready(() => ({ status: 401, error: "Incorrect API key provided" }));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute");
  await ai.until(() => ai.osd()?.startsWith("Vestra · Could not reach"), "error OSD");
  assert.match(ai.osd(), /^Vestra · Could not reach Custom · OpenAI format: .*Incorrect API key provided/);
  assert.deepEqual(host.osdActions.at(-1), []);
  assert.ok(!host.coreRuns.includes("mute"));
  await ai.until(() => !host.paletteLead.status, "status cleared");
});

test("the model answers without steps: its sentence is shown", async (t) => {
  const ai = await ready(() => ({ text: ["No command changes the picture's warmth."] }));
  t.after(() => ai.stop());
  ai.host.dialogAnswers.push(CONTINUE);
  await ai.host.runPaletteLead("warmer");
  await ai.until(() => ai.osd() === "Vestra · No command changes the picture's warmth.", "refusal");
});

test("a plan the plugin refuses runs no step: another step kind (the old speed step), too many steps", async (t) => {
  const many = Array.from({ length: 9 }, () => MUTE);
  const ai = await ready(byCall(plan(MUTE, { kind: "speed", rate: 1.5 }), plan(...many)));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("mute and faster");
  await ai.until(() => ai.osd() === "Vestra · Step 2 failed: Vestra cannot do “speed”", "unknown kind");
  assert.equal(host.source.rate, 1);
  assert.ok(!host.coreRuns.includes("mute"), "checked before any step runs");
  await host.runPaletteLead("mute nine times");
  await ai.until(() => ai.osd() === "Vestra · The plan has more than 8 steps", "too many");
  assert.ok(!host.coreRuns.includes("mute"));
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
  await ai.until(() => ai.osd() === `Vestra · ${ai.mute}`, "result");
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
