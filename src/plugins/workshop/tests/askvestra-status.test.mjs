// Ask Vestra's status texts fit the player's limit in every language
// (askvestra/status.ts): the table of each template's words per language
// matches the catalogs, and `fit` cuts the data so that the shown text,
// translated and filled in the way the player does it, is at most 80
// characters as the player counts them (Unicode scalar values), with a
// trailing "…".
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { LIST_SEPARATORS, LOCALES, STATUS_CHARS, WORDS, fit } from "../askvestra/status.ts";

const locales = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "locales");
/** Every catalog by language, and English (the source texts). */
const catalogs = new Map([["en", new Map()]]);
for (const file of fs.readdirSync(locales).filter((name) => name.endsWith(".po"))) {
  const messages = new Map();
  for (const m of fs.readFileSync(path.join(locales, file), "utf8").matchAll(/^msgid (".*")\r?\nmsgstr (".*")$/gm))
    messages.set(JSON.parse(m[1]), JSON.parse(m[2]));
  catalogs.set(path.basename(file, ".po"), messages);
}
/** The player's "list" separators (host i18n/text.rs SEPARATORS). */
const SEPARATORS = { en: ", ", "zh-Hans": "、", ja: "、", ko: ", ", de: ", ", fr: ", ", es: ", " };
const chars = (text) => [...text].length;
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

test("the catalogs are the player's languages; the list separators are the player's", () => {
  assert.deepEqual([...catalogs.keys()].sort(), [...LOCALES].sort());
  assert.deepEqual(LIST_SEPARATORS, SEPARATORS);
});

test("WORDS: each template's own words in each language (a catalog change fails here until the table follows)", () => {
  for (const [source, row] of Object.entries(WORDS)) {
    assert.equal(row.length, LOCALES.length, source);
    LOCALES.forEach((language, index) => {
      const translated = language === "en" ? source : catalogs.get(language).get(source);
      assert.ok(translated, `${language}: no translation of “${source}”`);
      assert.equal(row[index], chars(translated.replace(PLACEHOLDER, "")), `${language}: ${source}`);
    });
  }
});

const long = "x".repeat(200);
const label = (i) => `Some Plugin With A Long Name: A Very Long Command Title ${i}`;
const cases = {
  "the model's sentence": long,
  "a sentence of emoji and CJK (counted as the player counts)": "🎬播放".repeat(60),
  "the result list": { source: "Done · {}", args: [{ list: Array.from({ length: 8 }, (_, i) => label(i)) }] },
  "an opened window": { source: "Opened {}", args: [long] },
  "a step failed with the command's reason": { source: "Step {0} failed: {1}", args: [8, long] },
  "a step failed: busy": { source: "Step {0} failed: {1}", args: [8, { source: "{} is busy", args: [long] }] },
  "a step failed: no such command": { source: "Step {0} failed: {1}", args: [8, { source: "There is no command “{}”", args: [long] }] },
  "a step failed: another kind": { source: "Step {0} failed: {1}", args: [8, { source: "Vestra cannot do “{}”", args: [long] }] },
  "a step failed: the player lets no command run": { source: "Step {0} failed: {1}", args: [8, { source: "This player does not let Vestra run commands" }] },
  "a provider error": { source: "Could not reach {0}: {1}", args: [{ source: "Custom · Anthropic format" }, long] },
  "a provider error, long provider name": { source: "Could not reach {0}: {1}", args: [long, long] },
  "a declined step": { source: "Stopped: {} was not run", args: [long] },
  "the row while a step runs": { source: "Vestra is working · Step {0} of {1} · {2}", args: [8, 8, long] },
  "the key cannot be read": { source: "Cannot read the API key; save it again in the settings" },
};

test("fit: every status text is at most 80 characters in every language, cut with a trailing …", () => {
  for (const [name, line] of Object.entries(cases)) {
    for (const language of LOCALES) {
      const text = render(fit(line, language), language);
      assert.ok(chars(text) >= 1 && chars(text) <= STATUS_CHARS, `${name} (${language}): ${chars(text)} characters: ${text}`);
      if (typeof line === "string" || JSON.stringify(line).includes("x".repeat(100))) assert.ok(text.includes("…"), `${name} (${language}): cut with …`);
    }
  }
});

test("fit: a text that fits stays as it is; a cut one fills the room exactly", () => {
  const short = { source: "Done · {}", args: [{ list: ["Vestra Player: Mute", "Rotate: 90°"] }] };
  assert.deepEqual(fit(short, "en"), short);
  assert.equal(fit("Fine.", "en"), "Fine.");
  const sentence = fit(long, "en");
  assert.equal(chars(sentence), STATUS_CHARS);
  assert.ok(sentence.endsWith("…"));
  for (const language of LOCALES) assert.equal(chars(render(fit(cases["a provider error"], language), language)), STATUS_CHARS, language);
  // The list keeps whole labels while they fit, ends with the cut one and drops the rest.
  const labels = Array.from({ length: 8 }, (_, i) => `Vestra Player: Command ${i}`);
  const done = fit({ source: "Done · {}", args: [{ list: labels }] }, "en");
  const items = done.args[0].list;
  assert.ok(items.length > 1 && items.length < 8, JSON.stringify(items));
  assert.deepEqual(items.slice(0, -1), labels.slice(0, items.length - 1));
  assert.ok(items.at(-1).endsWith("…"));
  assert.ok(chars(render(done, "en")) <= STATUS_CHARS);
  // A code point is never split (an emoji is one character, as for the player).
  assert.ok(!/[\uD800-\uDBFF]$/.test(fit("🎬".repeat(100), "en").replace(/…$/, "")));
});
