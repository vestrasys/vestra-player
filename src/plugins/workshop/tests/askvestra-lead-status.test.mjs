// Ask Vestra's palette status with the player's 1.8.0 additions
// (doc/architecture/palette-lead.md; SDK 1.8.0): Stop ends as the state
// `stopped` (the three Stop paths; every other note stays a `note`); while
// the model plans the row says "Vestra is working · Planning…" and the
// strip, through `status.strip`, "Planning…" (no `strip` once there are
// steps: the player writes the strip then); a working text is fitted to 80
// characters, an end state to 160 (the strip's two lines).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import { STEP_GAP_MS } from "../askvestra/ask.ts";
import { END_STATUS_CHARS, LOCALES, STATUS_CHARS, WORDS, fit } from "../askvestra/status.ts";
import { server, startAi } from "./fixture.mjs";
import { shown } from "./lead-status.mjs";

const plan = (...steps) => ({ toolCalls: [{ name: "run_steps", args: { steps } }] });
const command = (id) => ({ kind: "command", id });
const CONTINUE = { confirmed: true, checked: false };
const CANCEL = { confirmed: false, checked: false };
const chars = (text) => [...text].length;

/** The end states the plugin sent, as "state: text" in the host's language. */
const ends = (host) =>
  host
    .sent("palette.lead.set")
    .filter((p) => p.status?.state && p.status.state !== "working")
    .map((p) => `${p.status.state}: ${shown(host, p.status.text)}`);

// What the player would do with every status sent (the 1.8.0 rule): a
// working text and `strip` 1–80 characters, an end state's text 1–160,
// `strip` only with a working status; and the plugin logged no refused row.
const hosts = [];
const refused = [];
const original = console.error;
console.error = (...args) => {
  const line = args.map(String).join(" ");
  if (line.includes("lead row: ")) refused.push(line);
  original(...args);
};
test.afterEach(() => {
  const seen = hosts.splice(0);
  assert.deepEqual(refused.splice(0), [], "the player refused a lead row");
  for (const host of seen)
    for (const { status } of host.sent("palette.lead.set")) {
      if (!status) continue;
      const end = !!status.state && status.state !== "working";
      const text = shown(host, status.text);
      const max = end ? END_STATUS_CHARS : STATUS_CHARS;
      assert.ok(chars(text) >= 1 && chars(text) <= max && !/[\0\n\r]/.test(text), `status.text the player refuses (${chars(text)} characters, ${status.state ?? "working"}): ${text}`);
      if (status.strip === undefined) continue;
      assert.ok(!end, `status.strip with the end state ${status.state}`);
      const strip = shown(host, status.strip);
      assert.ok(chars(strip) >= 1 && chars(strip) <= STATUS_CHARS && !/[\0\n\r]/.test(strip), `status.strip the player refuses: ${strip}`);
    }
});

/** The plugin as a built-in, a mock model set up, a file playing. */
async function ready(reply, options = {}) {
  const mock = await server(reply);
  const ai = await startAi({ compat: mock.url, builtin: true, ...options });
  hosts.push(ai.host);
  ai.host.openElsewhere("/media/Secret Holiday Film.mkv");
  ai.host.setSource({ duration: 600, position: 30, paused: false });
  await ai.host.settle();
  return { ...ai, mock, end: () => ends(ai.host).at(-1), ends: () => ends(ai.host) };
}

// ---- the limits and the table

test("the limits are the player's (the TestHost's): 80 for a working text and the strip, 160 for an end state", async (t) => {
  assert.equal(STATUS_CHARS, 80);
  assert.equal(END_STATUS_CHARS, 160);
  // A built-in plugin of its own that holds the lead row and sets what the test gives it.
  const host = new TestHost();
  let plugin;
  const started = await startPlugin((given) => void (plugin = given), { directory: new URL("..", import.meta.url), host, builtin: true });
  t.after(() => started.stop());
  const row = { title: "Ask Vestra", icon: "ai" };
  const lead = plugin.palette.lead(row);
  await lead.ready;
  const set = (status) => lead.update({ ...row, status });
  const x = (length) => "x".repeat(length);
  const refusedWith = (limit) => (error) => error.code === "invalid" && error.message.includes(`must be 1–${limit} characters on one line`);
  // A working text: 80 is accepted, 81 refused.
  await set({ text: x(STATUS_CHARS) });
  await assert.rejects(set({ text: x(STATUS_CHARS + 1) }), refusedWith(80));
  // The strip's text: 80 is accepted, 81 refused.
  await set({ text: "Working", strip: x(STATUS_CHARS) });
  await assert.rejects(set({ text: "Working", strip: x(STATUS_CHARS + 1) }), refusedWith(80));
  // Every end state: 160 is accepted, 161 refused; `strip` is refused with one.
  for (const state of ["done", "failed", "note", "stopped"]) {
    await set({ text: x(END_STATUS_CHARS), state });
    assert.equal(host.paletteLead.status.state, state);
    await assert.rejects(set({ text: x(END_STATUS_CHARS + 1), state }), refusedWith(160), state);
    await assert.rejects(set({ text: "Ended", strip: "Ended", state }), (error) => error.code === "invalid", state);
  }
});

const locales = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "locales");
const catalog = (language) => {
  const messages = new Map();
  for (const m of fs.readFileSync(path.join(locales, `${language}.po`), "utf8").matchAll(/^msgid (".*")\r?\nmsgstr (".*")$/gm)) messages.set(JSON.parse(m[1]), JSON.parse(m[2]));
  return messages;
};
const catalogs = new Map(LOCALES.filter((language) => language !== "en").map((language) => [language, catalog(language)]));
const SEPARATORS = { en: ", ", "zh-Hans": "、", ja: "、", ko: ", ", de: ", ", fr: ", ", es: ", " };
const PLACEHOLDER = /\{\d*\}/g;
/** A line as the player shows it in `language`. */
function render(line, language) {
  if (typeof line === "string") return line;
  if (typeof line === "number") return String(line);
  if ("list" in line) return line.list.map((item) => render(item, language)).join(SEPARATORS[language]);
  const template = (language === "en" ? null : catalogs.get(language).get(line.source)) || line.source;
  const args = (line.args ?? []).map((arg) => render(arg, language));
  let next = 0;
  return template.replace(PLACEHOLDER, (placeholder) => {
    const index = placeholder.slice(1, -1);
    return args[index === "" ? next++ : Number(index)];
  });
}

test("WORDS: the row's planning text is a template of every catalog and fits a working status as it is", () => {
  const source = "Vestra is working · Planning…";
  assert.ok(source in WORDS);
  for (const [index, language] of LOCALES.entries()) {
    const text = render({ source }, language);
    if (language !== "en") assert.ok(catalogs.get(language).get(source), `${language}: no translation`);
    assert.equal(WORDS[source][index], chars(text), language);
    assert.ok(chars(text) <= STATUS_CHARS, `${language}: ${text}`);
    assert.deepEqual(fit({ source }, language), { source });
    // The row says the strip's word after the row's own: "… · Planning…".
    assert.ok(text.endsWith(render({ source: "Planning…" }, language)), `${language}: ${text}`);
  }
});

const long = "x".repeat(400);
const label = (i) => `Some Plugin With A Long Name: A Very Long Command Title ${i}`;
const endCases = {
  "the model's sentence": long,
  "a sentence of emoji and CJK (counted as the player counts)": "🎬播放".repeat(100),
  "the result list": { source: "Done · {}", args: [{ list: Array.from({ length: 8 }, (_, i) => label(i)) }] },
  "an opened window": { source: "Opened {}", args: [long] },
  "a step failed with the command's reason": { source: "Step {0} failed: {1}", args: [8, long] },
  "a step failed: no such command": { source: "Step {0} failed: {1}", args: [8, { source: "There is no command “{}”", args: [long] }] },
  "a provider error": { source: "Could not reach {0}: {1}", args: [{ source: "Custom · Anthropic format" }, long] },
  "a provider error, long provider name": { source: "Could not reach {0}: {1}", args: [long, long] },
  "a declined step": { source: "Stopped: {} was not run", args: [long] },
};

test("fit with the end-state limit: every end text is at most 160 characters in every language, cut with a trailing …", () => {
  for (const [name, line] of Object.entries(endCases))
    for (const language of LOCALES) {
      const text = render(fit(line, language, END_STATUS_CHARS), language);
      assert.ok(chars(text) > STATUS_CHARS && chars(text) <= END_STATUS_CHARS, `${name} (${language}): ${chars(text)} characters: ${text}`);
      assert.ok(text.includes("…"), `${name} (${language}): cut with …`);
    }
  for (const language of LOCALES) assert.equal(chars(render(fit(endCases["a provider error"], language, END_STATUS_CHARS), language)), END_STATUS_CHARS, language);
});

test("fit with the end-state limit: a text between 81 and 160 characters stays whole; without a limit it is a working text (80)", () => {
  const sentence = "y".repeat(120);
  assert.equal(fit(sentence, "en", END_STATUS_CHARS), sentence);
  assert.equal(chars(fit(sentence, "en")), STATUS_CHARS);
  const cut = fit(long, "en", END_STATUS_CHARS);
  assert.equal(chars(cut), END_STATUS_CHARS);
  assert.ok(cut.endsWith("…"));
  // Eight labels of a result that did not fit 80 characters fit 160 whole.
  const labels = Array.from({ length: 8 }, (_, i) => `Player: Step ${i}`);
  const done = { source: "Done · {}", args: [{ list: labels }] };
  assert.ok(fit(done, "en").args[0].list.length < 8);
  assert.deepEqual(fit(done, "en", END_STATUS_CHARS), done);
});

// ---- planning: the row's text and the strip's

test("planning: the row says Vestra is working · Planning…, the strip Planning… (status.strip), in the interface language too", async () => {
  for (const [locale, row, strip] of [
    ["en", "Vestra is working · Planning…", "Planning…"],
    ["zh-Hans", "Vestra 正在处理 · 规划中…", "规划中…"],
  ]) {
    const ai = await ready(() => ({ hang: true }), { locale });
    try {
      const { host, mock } = ai;
      host.dialogAnswers.push(CONTINUE);
      await host.runPaletteLead("mute");
      await ai.until(() => mock.chats.length === 1 && !!host.paletteLead.status, `${locale}: planning`);
      const { status } = host.paletteLead;
      assert.equal(shown(host, status.text), row);
      assert.equal(shown(host, status.strip), strip);
      assert.equal(status.state, undefined, "working");
      assert.ok(status.note, "Enter stops");
      await host.command("askvestra.stop").run();
      await ai.until(() => !!ai.end() && !host.paletteLead.status, `${locale}: stopped`);
    } finally {
      await ai.stop();
    }
  }
});

test("once there are steps the status has no strip: the player writes the strip", async (t) => {
  const steps = Array.from({ length: 3 }, () => command("host/volume-up"));
  const ai = await ready(() => plan(...steps));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("louder three times");
  await ai.until(() => !host.paletteLead.status && !!ai.end(), "result", 30_000);
  const working = host
    .sent("palette.lead.set")
    .map((p) => p.status)
    .filter((status) => status && !status.state);
  const planning = working.filter((status) => shown(host, status.text) === "Vestra is working · Planning…");
  const stepping = working.filter((status) => shown(host, status.text).startsWith("Vestra is working · Step "));
  assert.ok(planning.length >= 1 && stepping.length >= 1, JSON.stringify(working));
  assert.equal(planning.length + stepping.length, working.length, "no other working text");
  for (const status of planning) assert.equal(shown(host, status.strip), "Planning…");
  for (const status of stepping) assert.ok(!("strip" in status), JSON.stringify(status));
  // An end state has no strip either: its text is the strip's.
  for (const p of host.sent("palette.lead.set")) if (p.status?.state) assert.ok(!("strip" in p.status), JSON.stringify(p.status));
  assert.match(ai.end(), /^done: Done · /);
});

// ---- Stop: the state `stopped`

test("Stop while planning (Enter on the status row, and the palette command) ends as stopped: Stopped", async () => {
  for (const stop of [(host) => host.runPaletteLead("", { status: true }), (host) => host.command("askvestra.stop").run()]) {
    const ai = await ready(() => ({ hang: true }));
    try {
      const { host, mock } = ai;
      host.dialogAnswers.push(CONTINUE);
      await host.runPaletteLead("mute");
      await ai.until(() => mock.chats.length === 1 && !!host.paletteLead.status, "planning");
      await stop(host);
      await ai.until(() => !!ai.end() && !host.paletteLead.status, "stopped");
      assert.deepEqual(ai.ends(), ["stopped: Stopped"]);
      assert.ok(!host.coreRuns.includes("mute"));
    } finally {
      await ai.stop();
    }
  }
});

test("Stop between steps ends as stopped: Stopped", async (t) => {
  const steps = Array.from({ length: 8 }, () => command("host/volume-up"));
  const ai = await ready(() => plan(...steps));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("louder eight times");
  await ai.until(() => host.coreRuns.filter((run) => run === "volume-up").length === 2, "two steps", 30_000);
  await host.command("askvestra.stop").run();
  await ai.until(() => !!ai.end() && !host.paletteLead.status, "stopped");
  assert.deepEqual(ai.ends(), ["stopped: Stopped"]);
});

test("Stop while the consent is asked (a request that waits) ends as stopped: Stopped, once", async (t) => {
  const ai = await ready(() => plan(command("host/mute")));
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
  assert.equal(shown(host, host.paletteLead.status.strip), "Planning…");
  void host.runPaletteLead("mute", { keep: true, status: true });
  await ai.until(() => !host.paletteLead.status, "status cleared");
  answer();
  await host.settle();
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.equal(mock.chats.length, 0, "nothing sent");
  assert.deepEqual(ai.ends(), ["stopped: Stopped"]);
});

test("Don't Run on the sheet ends as stopped: Stopped: nothing was run", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("host/stop"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  host.dialogAnswers.push(CONTINUE, CANCEL); // consent, Don't Run on the sheet
  await host.runPaletteLead("louder, close, mute");
  await ai.until(() => !!ai.end() && !host.paletteLead.status, "declined");
  await new Promise((resolve) => setTimeout(resolve, 2 * STEP_GAP_MS));
  assert.deepEqual(ai.ends(), ["stopped: Stopped: nothing was run"]);
});

test("Don't Run when a later step asks again ends as stopped: Stopped: … was not run", async (t) => {
  const ai = await ready(() => plan(command("host/volume-up"), command("host/mute")));
  t.after(() => ai.stop());
  const { host } = ai;
  const { commands } = await ai.plugin.commands.list();
  const mute = commands.find((row) => row.id === "host/mute").label;
  // The first step runs; the second is declined (the player asked again for it).
  const ran = [];
  host.handle("commands.run", (p) => {
    if (p.id === "host/mute") return { ran: false, declined: true };
    ran.push(p.id);
    return { ran: true };
  });
  host.dialogAnswers.push(CONTINUE);
  await host.runPaletteLead("louder and mute");
  await ai.until(() => !!ai.end() && !host.paletteLead.status, "declined", 30_000);
  assert.deepEqual(ran, ["host/volume-up"]);
  assert.deepEqual(ai.ends(), [`stopped: Stopped: ${mute} was not run`]);
});

// ---- the other ends keep their states; an end text may be 160 characters

test("the model's sentence stays a note and is cut to the end state's 160 characters, in the interface language too", async () => {
  for (const locale of ["en", "zh-Hans"]) {
    const sentence = `${"没有命令可以把音量设为某个具体数值。".repeat(6)} ${"No command sets an exact volume. ".repeat(6)}`;
    const ai = await ready(() => ({ text: [sentence] }), { locale });
    try {
      ai.host.dialogAnswers.push(CONTINUE);
      await ai.host.runPaletteLead("volume 37");
      await ai.until(() => !!ai.end(), `${locale}: sentence`);
      assert.ok(ai.end().startsWith("note: "), ai.end());
      const text = ai.end().slice("note: ".length);
      assert.equal(chars(text), END_STATUS_CHARS, text);
      assert.ok(text.endsWith("…"));
      assert.ok(sentence.startsWith(text.slice(0, -1)));
      await ai.until(() => !ai.host.paletteLead.status, `${locale}: idle`);
    } finally {
      await ai.stop();
    }
  }
});

test("a sentence of 81 to 160 characters is shown whole (it was cut at 80)", async (t) => {
  const sentence = "No command sets an exact volume; use Volume Up or Volume Down several times, or drag the volume slider in the bottom bar.";
  assert.ok(chars(sentence) > STATUS_CHARS && chars(sentence) <= END_STATUS_CHARS);
  const ai = await ready(() => ({ text: [sentence] }));
  t.after(() => ai.stop());
  ai.host.dialogAnswers.push(CONTINUE);
  await ai.host.runPaletteLead("volume 37");
  await ai.until(() => !!ai.end(), "sentence");
  assert.deepEqual(ai.ends(), [`note: ${sentence}`]);
});

test("a model error stays a note, a result stays done", async (t) => {
  const failing = await ready(() => ({ status: 401, error: "Incorrect API key provided" }));
  t.after(() => failing.stop());
  failing.host.dialogAnswers.push(CONTINUE);
  await failing.host.runPaletteLead("mute");
  await failing.until(() => !!failing.end(), "error");
  assert.match(failing.end(), /^note: Could not reach Custom · OpenAI format: \S/);
  const fine = await ready(() => plan(command("host/mute")));
  t.after(() => fine.stop());
  fine.host.dialogAnswers.push(CONTINUE);
  await fine.host.runPaletteLead("mute");
  await fine.until(() => !!fine.end() && !fine.host.paletteLead.status, "result");
  assert.match(fine.end(), /^done: Done · /);
});
