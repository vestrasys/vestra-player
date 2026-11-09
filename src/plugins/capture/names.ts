import { clock, stemOf } from "./model.ts";

/**
 * File-name templates (proposal §3.7, mpv's `screenshot-template`): one for
 * frames and one for a burst's folder.
 */
export type TemplateKind = "image" | "burst";
export const DEFAULT_TEMPLATES: Readonly<Record<TemplateKind, string>> = {
  image: "{name}_{pos}",
  burst: "Capture-{date} {time}",
};
/** The variables, in the order the settings page's chips show them. */
export const VARIABLES = ["{name}", "{pos}", "{start}", "{end}", "{date}", "{time}", "{n}"] as const;
/** At most this many characters per template (a name stays well inside the 255 of the file systems). */
export const TEMPLATE_LIMIT = 120;

export interface NameValues {
  /** The playing source (its file name without the extension is `{name}`). */
  source: string | null;
  /** The frame time (`{pos}`), and a burst's first and last frame times. */
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
/**
 * Windows device names (`CON`, `NUL`, `COM1`, `aux.png` …) open the device
 * instead of a file: the part before the first `.` is checked, as Windows
 * does, in any case, and gets a `_`.
 */
const RESERVED = /^(con|prn|aux|nul|com[0-9¹²³]|lpt[0-9¹²³]|conin\$|conout\$)$/i;
export const safeName = (text: string) => {
  const name = text.replace(ILLEGAL, "_").replace(/[. ]+$/, "").trim();
  const dot = name.indexOf(".");
  const head = dot < 0 ? name : name.slice(0, dot);
  return RESERVED.test(head.trimEnd()) ? `${head}_${dot < 0 ? "" : name.slice(dot)}` : name;
};
/**
 * At most this many characters per rendered name, so that the name with its
 * extension and a ` (n)` stays within 180 (inside the 255 of the file systems,
 * and short enough for Windows paths in deep folders).
 */
export const NAME_LIMIT = 168;

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
  const fill = (text: string) => {
    const render = () => safeName(text.replace(/\{[a-z]+\}/g, (word) => table[word] ?? word));
    let name = render();
    // Too long: the source's name gives way first, then the end of the whole name.
    const over = name.length - NAME_LIMIT;
    const uses = text.split("{name}").length - 1;
    if (over > 0 && uses > 0) {
      const stem = table["{name}"]!;
      table["{name}"] = stem.slice(0, Math.max(1, stem.length - Math.ceil(over / uses))).trimEnd();
      name = render();
      table["{name}"] = stem;
    }
    return name.length > NAME_LIMIT ? safeName(name.slice(0, NAME_LIMIT)) : name;
  };
  return fill(normalTemplate(kind, template)) || fill(DEFAULT_TEMPLATES[kind]);
}

/** Insert a variable chip at the end of a template (the settings page's chips; a click puts it where the text ends). */
export const insertVariable = (template: string, variable: string) =>
  (VARIABLES as readonly string[]).includes(variable) ? `${template}${variable}`.slice(0, TEMPLATE_LIMIT) : template;
