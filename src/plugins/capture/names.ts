import { clock, stemOf } from "./model.ts";

/**
 * File-name templates (proposal §3.7, mpv's `screenshot-template`): one for
 * frames, one for clips and one for a burst's folder. The defaults name files
 * as the plugin always did, except that a clip also states its end.
 */
export type TemplateKind = "image" | "clip" | "burst";
export const DEFAULT_TEMPLATES: Readonly<Record<TemplateKind, string>> = {
  image: "{name}_{pos}",
  clip: "{name}_{start}-{end}",
  burst: "Capture-{date} {time}",
};
/** The variables, in the order the tab's chips show them. */
export const VARIABLES = ["{name}", "{pos}", "{start}", "{end}", "{date}", "{time}", "{n}"] as const;
/** At most this many characters per template (a name stays well inside the 255 of the file systems). */
export const TEMPLATE_LIMIT = 120;

export interface NameValues {
  /** The playing source (its file name without the extension is `{name}`). */
  source: string | null;
  /** The frame time (`{pos}`), and the clip's or burst's first and last times. */
  start: number;
  end?: number;
  /** When the capture was made (`{date}`, `{time}`, local time). */
  now: Date;
  /** A running number (`{n}`, three digits). */
  n?: number;
}

const two = (n: number) => String(n).padStart(2, "0");
/** `2026-11-07`, local time. */
export const dateOf = (now: Date) => `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
/** `16.21.05`, local time (no colons: they are not allowed in Windows file names). */
export const timeOf = (now: Date) => `${two(now.getHours())}.${two(now.getMinutes())}.${two(now.getSeconds())}`;

/** Characters no file system takes in a name, and control characters, become `_`. */
const ILLEGAL = /[\\/:*?"<>|\u0000-\u001f\u007f]/g;
export const safeName = (text: string) => text.replace(ILLEGAL, "_").replace(/[. ]+$/, "").trim();

/** A template as kept: at most {@link TEMPLATE_LIMIT} characters, the default when it is blank. */
export function normalTemplate(kind: TemplateKind, value: unknown) {
  const text = typeof value === "string" ? value.slice(0, TEMPLATE_LIMIT) : "";
  return text.trim() ? text : DEFAULT_TEMPLATES[kind];
}

/**
 * The name a template gives, without the extension. Every variable works in
 * every template (`{pos}` and `{start}` are the first time, `{end}` the last;
 * a frame's three are the same). An unknown `{word}` stays as written. Illegal
 * characters become `_`; a name that comes out empty uses the default template.
 */
export function renderName(kind: TemplateKind, template: string, values: NameValues): string {
  const end = values.end ?? values.start;
  const table: Record<string, string> = {
    "{name}": stemOf(values.source),
    "{pos}": clock(values.start, "."),
    "{start}": clock(values.start, "."),
    "{end}": clock(end, "."),
    "{date}": dateOf(values.now),
    "{time}": timeOf(values.now),
    "{n}": String(values.n ?? 1).padStart(3, "0"),
  };
  const fill = (text: string) => safeName(text.replace(/\{[a-z]+\}/g, (word) => table[word] ?? word));
  return fill(normalTemplate(kind, template)) || fill(DEFAULT_TEMPLATES[kind]);
}

/** Insert a variable chip at the end of a template (the tab's chips; a click puts it where the text ends). */
export const insertVariable = (template: string, variable: string) =>
  (VARIABLES as readonly string[]).includes(variable) ? `${template}${variable}`.slice(0, TEMPLATE_LIMIT) : template;
