// The palette row's status text, which the player also shows in its status
// strip (Ask Vestra UX PROPOSAL §4.2), fitted to the player's limit: 1–80
// characters as shown, translated and filled in, counted as Unicode scalar
// values (host `capabilities/palette_lead.rs` `text`: `chars().count()`;
// JavaScript's string iterator counts the same). A longer text is refused
// (`invalid`) and the user would see nothing, so the data in it (labels,
// the model's sentence, error messages) is cut with a trailing "…".
//
// The plugin hands the player templates, not translated strings, so it
// counts each template's own words in the interface language from a table
// (WORDS; `tests/askvestra-status.test.mjs` checks it against every
// catalog). The data then gets what is left.
import type { I18n, Locale, Text, TextArg } from "@vestra-player/sdk";

/** The player's limit for `status.text` (and the strip's text). */
export const STATUS_CHARS = 80;

/** The player's languages, in the order of WORDS' rows. */
export const LOCALES = ["en", "zh-Hans", "ja", "ko", "de", "fr", "es"] as const satisfies readonly Locale[];

/** The player's list separator per language (host `i18n/text.rs` SEPARATORS, style "list"). */
export const LIST_SEPARATORS: Readonly<Record<Locale, string>> = { en: ", ", "zh-Hans": "、", ja: "、", ko: ", ", de: ", ", fr: ", ", es: ", " };

/**
 * Each status template's own words: the characters its text takes without
 * its placeholders, per language in LOCALES' order.
 */
export const WORDS = {
  "Planning…": [9, 4, 4, 5, 6, 14, 13],
  "Vestra is working · Step {0} of {1} · {2}": [32, 22, 23, 24, 34, 33, 37],
  Stopped: [7, 3, 6, 3, 8, 6, 8],
  "Stopped: nothing was run": [24, 12, 19, 20, 36, 29, 28],
  "Stopped: {} was not run": [21, 8, 20, 20, 33, 29, 24],
  "Done · {}": [7, 5, 5, 5, 9, 10, 8],
  "Opened {}": [7, 4, 7, 10, 9, 7, 9],
  "Step {0} failed: {1}": [14, 7, 15, 7, 25, 20, 16],
  "Took too long": [13, 4, 11, 15, 21, 9, 15],
  "Could not reach {0}: {1}": [18, 6, 11, 14, 19, 25, 27],
  "The model gave no steps": [23, 8, 17, 18, 39, 32, 28],
  "The plan has more than {} steps": [29, 7, 17, 12, 31, 30, 27],
  "Cannot read the API key; save it again in the settings": [54, 21, 28, 31, 71, 76, 66],
  "{} is no longer there": [19, 5, 9, 14, 19, 14, 13],
  "Too many commands at once": [25, 9, 17, 21, 27, 27, 28],
  "The request has ended": [21, 5, 9, 9, 21, 23, 24],
  "This player does not let Vestra run commands": [44, 19, 30, 32, 50, 58, 55],
  "Vestra cannot do “{}”": [19, 13, 18, 23, 20, 29, 24],
  "There is no command “{}”": [22, 6, 12, 11, 24, 29, 24],
  // Provider names in "Could not reach {0}: {1}" (main.ts providerName).
  "Custom · OpenAI format": [22, 15, 16, 18, 33, 28, 30],
  "Custom · Anthropic format": [25, 18, 19, 21, 36, 31, 33],
} as const satisfies Record<string, readonly number[]>;

/** A status template: the player translates it (and the catalogs' extraction reads this union). */
export type Source = keyof typeof WORDS;

/**
 * A status text before translation: data (a string, cut when needed; a
 * number, kept), a template of WORDS with its arguments, or a list the
 * player joins.
 */
export type Line = string | number | { source: Source; args?: Line[] } | { list: Line[] };

const chars = (text: string) => [...text].length;

/** A template's own words in `locale` (an unknown language: the longest). */
const words = (source: Source, locale: Locale): number => {
  const row: readonly number[] = WORDS[source];
  const index = (LOCALES as readonly string[]).indexOf(locale);
  return index >= 0 ? row[index]! : Math.max(...row);
};

/** The characters that cannot be cut in `locale`: template words, numbers and list separators. */
const fixed = (line: Line, locale: Locale): number => {
  if (typeof line === "string") return 0;
  if (typeof line === "number") return chars(String(line));
  if ("list" in line)
    return Math.max(0, line.list.length - 1) * chars(LIST_SEPARATORS[locale] ?? ", ") + line.list.reduce<number>((sum, item) => sum + fixed(item, locale), 0);
  return words(line.source, locale) + (line.args ?? []).reduce<number>((sum, arg) => sum + fixed(arg, locale), 0);
};

/** The data of a line as one string: for measuring a list item. */
const data = (line: Line): string => {
  if (typeof line === "string") return line;
  if (typeof line === "number") return "";
  if ("list" in line) return line.list.map(data).join("");
  return (line.args ?? []).map(data).join("");
};

/**
 * `line` with its data cut so that, shown in `locale`, it is at most `max`
 * characters: data in order gets what the words leave; the piece that does
 * not fit ends with "…" and data after it is empty; a list ends with its
 * cut item (only the separators of the items kept are shown).
 */
export function fit(line: Line, locale: Locale, max = STATUS_CHARS): Line {
  let room = max - fixed(line, locale);
  const cut = (part: Line): Line => {
    if (typeof part === "number") return part;
    if (typeof part === "string") {
      const length = chars(part);
      if (length <= room) {
        room -= length;
        return part;
      }
      const kept = room > 0 ? `${[...part].slice(0, room - 1).join("")}…` : "";
      room = 0;
      return kept;
    }
    if ("list" in part) {
      const items: Line[] = [];
      for (const item of part.list) {
        // Nothing left: the items after go, with their separators.
        if (room === 0 && items.length) break;
        const before = room;
        items.push(cut(item));
        // This item was cut (it ends with "…"): the items after it go.
        if (before - room < chars(data(item))) break;
      }
      return { list: items };
    }
    return part.args ? { source: part.source, args: part.args.map(cut) } : { source: part.source };
  };
  return cut(line);
}

/** The value the SDK sends: templates the player translates, data as it is. */
export function build(i18n: I18n, line: Line): TextArg {
  if (typeof line === "string" || typeof line === "number") return line;
  if ("list" in line) return i18n.list(line.list.map((item) => build(i18n, item)));
  return i18n.text(line.source, (line.args ?? []).map((arg) => build(i18n, arg)));
}

/** A status text that fits the player's limit in the interface language, for `status.text`. */
export const statusText = (i18n: I18n, line: Line): string | Text => {
  const value = build(i18n, fit(line, i18n.locale));
  // A status is a template or a string; a number or a list alone is never one.
  return typeof value === "string" ? value : typeof value === "number" ? String(value) : (value as Text);
};
