/**
 * The player's interface languages: English (the source language) first,
 * then the translations, as the player's own table (its tests check that
 * they agree).
 */
export declare const LOCALES: readonly ["en", "zh-Hans", "ja", "ko", "de", "fr", "es"];
/** One message of a `.po` file (the header `msgid ""` is not one). */
export interface PoEntry {
    /** `msgctxt`, if any. */
    context?: string;
    /** `msgid`: the English source text. */
    id: string;
    /** `msgid_plural`, for a plural message. */
    plural?: string;
    /** `msgstr`, or `msgstr[0]`, `msgstr[1]`, … of a plural message. */
    strings: string[];
    /** Flagged `fuzzy`: left out of the compiled catalog. */
    fuzzy: boolean;
    /** `#.` lines: notes for translators. */
    notes: string[];
    /** `#:` references (`file:line`). */
    references: string[];
    /** Other `#,` flags than `fuzzy`. */
    flags: string[];
    /** A `#~` entry: no longer in the source. */
    obsolete: boolean;
}
/** The catalog key: `msgid`, or `msgctxt` U+0004 `msgid`. */
export declare const keyOf: (context: string | undefined, id: string) => string;
/** Parse a `.po` (or `.pot`) file. Throws with the line number on errors. */
export declare function parsePo(text: string): {
    header: string;
    entries: PoEntry[];
};
/** Write a `.po` file: the header, then the entries in the given order. */
export declare function writePo(header: string, entries: readonly PoEntry[]): string;
/** A message of a compiled catalog: a translation, or `[one, other]`. */
export type CompiledMessage = string | [string, string];
/**
 * The compiled catalog of one language (`locales/<lang>.json`): translated
 * entries only (not fuzzy, not obsolete, not empty), keys sorted. Plurals
 * have two forms, as the player shows them: `msgstr[0]` for `n == 1` and
 * `msgstr[1]` (else `msgstr[0]`) for every other `n`.
 */
export declare function compileCatalog(language: string, entries: readonly PoEntry[]): {
    language: string;
    messages: Record<string, CompiledMessage>;
};
/** Lookups in one language's messages, as the player does them. */
export declare class Catalog {
    readonly messages: ReadonlyMap<string, CompiledMessage>;
    constructor(messages?: Record<string, CompiledMessage>);
    /** From parsed `.po` entries. */
    static fromEntries(entries: readonly PoEntry[]): Catalog;
    /** The translation of `id` (the one form of a plural), or `undefined`. */
    lookup(id: string, context?: string): string | undefined;
    /** The translation of `id`, or `id` itself. */
    text(id: string): string;
    /**
     * A plugin sentence (`osd.show(template, args)`, dialogs): translated,
     * then formatted with `args` when given; without `args` the text shows
     * as it is, braces included.
     */
    template(text: string, args?: TemplateArgsValue): string;
    /** What an `@tr` call becomes, `undefined` when untranslated. */
    slint(call: TrCall): [string, string | undefined] | undefined;
}
/** Template arguments: positional (`{}`, `{0}`) or named (`{name}`). */
export type TemplateArgsValue = readonly (string | number)[] | Readonly<Record<string, string | number>> | null;
/**
 * Format a template the way Slint's `@tr` and the player do: `{}` takes
 * the next argument, `{0}` the first, `{name}` a field of an object; `{{`
 * and `}}` are braces. A placeholder without an argument stays as written.
 */
export declare function format(template: string, args: TemplateArgsValue): string;
/**
 * The placeholders of a template, sorted: `""` per `{}`, else the name or
 * number (`{n}` of a Slint plural is `"n"`). Translations must keep them.
 */
export declare function placeholders(text: string): string[];
/** One `@tr("context" => "id" | "plural" % n, args…)` call. */
export interface TrCall {
    context?: string;
    id: string;
    plural?: string;
    /** 1-based line of `@tr`. */
    line: number;
    /** Source offsets of the id literal (quotes included). */
    idSpan: [number, number];
    /** And of the plural literal. */
    pluralSpan?: [number, number];
}
/**
 * Every `@tr` call of a Slint source, in order; the same scanner as the
 * player's (both run the shared cases of the player's
 * `src/host/tests/fixtures/slint-tr.json`). Comments, strings and string
 * interpolation are skipped; a malformed call is left out.
 */
export declare function scanSlint(source: string): TrCall[];
/** A Slint string literal for `text`. */
export declare function slintQuote(text: string): string;
/** `source` with its `@tr` literals translated (what the player compiles). */
export declare function rewriteSlint(source: string, catalog: Catalog): string;
/**
 * The supported locale for a BCP 47 tag, as the player maps the system's
 * languages: Simplified Chinese is `zh-Hans`; Traditional Chinese (`zh-Hant`,
 * or Taiwan, Hong Kong, Macao) is English; other languages by their primary
 * subtag; `undefined` for a language the player does not have.
 */
export declare function matchLocale(tag: string): (typeof LOCALES)[number] | undefined;
/**
 * The player's built-in top-level menus: id → key (the English text paths
 * merge by), as the player's `MENU_ANCHORS`.
 */
export declare const MENU_KEYS: Readonly<Record<string, string>>;
/** The player's own submenus, by key path (their labels are the player's). */
export declare const HOST_GROUPS: string[];
/**
 * The keys a path merges by, as the player normalizes it: `@id` becomes the
 * built-in menu's key, the player's old Chinese titles of its own submenus
 * their keys.
 */
export declare function menuKeys(path: readonly string[]): string[];
/**
 * The player's own texts that the test host repeats (core actions, its
 * refusals, its menus), English → the player's Simplified Chinese
 * (`src/host/locales/zh-Hans.po` in the player; its tests check that they
 * agree).
 */
export declare const HOST_ZH_HANS: Readonly<Record<string, string>>;
