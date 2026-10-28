import type { ListStyle, Locale } from "./plugin/types.ts";
/**
 * The player's interface languages: English (the source language) first,
 * then the translations, as the player's own table (its tests check that
 * they agree).
 *
 * @since 1.5.9
 * @category Testing
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
    /** `# ` lines: the translator's own comments, kept as written. */
    comments?: string[];
}
/** The catalog key: `msgid`, or `msgctxt` U+0004 `msgid`. */
export declare const keyOf: (context: string | undefined, id: string) => string;
/** Parse a `.po` (or `.pot`) file. Throws with the line number on errors. */
export declare function parsePo(text: string): {
    header: string;
    /** The comment lines before the header entry, as written (`# Reviewed…`, `#, fuzzy`). */
    headerComments: string[];
    entries: PoEntry[];
};
/**
 * Write a `.po` file: the header (after its comment lines, as written),
 * then the entries in the given order.
 */
export declare function writePo(header: string, entries: readonly PoEntry[], headerComments?: readonly string[]): string;
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
/**
 * The ways a `TextList` is joined (`ListStyle`), as the player
 * joins them in each interface language (`LIST_SEPARATORS`).
 */
export declare const LIST_STYLES: readonly ListStyle[];
/**
 * The separator of each list style per interface language, as the player's
 * table (`i18n::text::SEPARATORS`; its tests check that they agree).
 *
 * @since 1.5.11
 * @category Testing
 */
export declare const LIST_SEPARATORS: Readonly<Record<Locale, Readonly<Record<ListStyle, string>>>>;
/**
 * The player's limits on one text value (a
 * {@link @vestra-player/sdk!Text | Text} or
 * {@link @vestra-player/sdk!TextList | TextList} with everything nested in it), as the player's
 * `i18n::text` (its tests check that they agree).
 *
 * @since 1.5.11
 * @category Testing
 */
export declare const TEXT_LIMITS: {
    /** Nesting depth of texts and lists; the value itself is depth 0 (8). */
    readonly depth: 8;
    /** Texts, lists and arguments in all, the value itself included (1024). */
    readonly nodes: 1024;
    /** Arguments of one text (16). */
    readonly args: 16;
    /** Items of one list (256). */
    readonly items: 256;
    /** UTF-8 bytes of a source text or a string argument (4096). */
    readonly stringBytes: 4096;
};
/** A translatable text on the wire: English source text and its arguments. */
export type TextJson = {
    $vestraText: string;
    args?: ArgJson[] | {
        [name: string]: ArgJson;
    };
};
/** A list on the wire: items joined the interface language's way. */
export type ListJson = {
    $vestraList: ArgJson[];
    style?: ListStyle;
};
/** An argument of a text: data (strings and numbers) or another text. */
export type ArgJson = string | number | TextJson | ListJson;
/** A text or list value (by its marker key). */
export declare function isTextValue(value: unknown): value is TextJson | ListJson;
/**
 * Why the player would refuse a text value or template arguments, or
 * `null`. `args` mode checks template arguments (an array or object of
 * arguments) instead of one value. The same rules and messages as the
 * player's `i18n::text::check`.
 */
export declare function textProblem(value: unknown, mode?: "value" | "args"): string | null;
type TemplateArgsLike = readonly ArgJson[] | Readonly<Record<string, ArgJson>> | null;
/**
 * A text value as the player shows it: the source translated with the
 * plugin's catalog and formatted with its (rendered) arguments; a list's
 * items joined with the locale's separator.
 */
export declare function renderText(value: TextJson | ListJson, catalog: Catalog, locale: string): string;
/**
 * Template arguments with their texts rendered (for `format`): what the
 * player fills a template with.
 */
export declare function renderTemplateArgs(args: TemplateArgsLike | undefined, catalog: Catalog, locale: string): TemplateArgsValue | undefined;
/** The English rendering of a text value: the key a menu merges by. */
export declare function textKey(value: TextJson | ListJson): string;
/**
 * A property value with every text value in it replaced by its shown
 * string, as the player does before it hands the value to Slint. Returns
 * a problem (`textProblem`) instead when a text value is malformed.
 */
export declare function resolveTexts<T>(value: T, catalog: Catalog, locale: string): {
    value: T;
} | {
    problem: string;
};
/** Whether a value holds a text value anywhere (a cheap pre-check). */
export declare function hasTexts(value: unknown): boolean;
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
/**
 * The catalog of a package directory for `locale`, as the player loads it:
 * the compiled `locales/<lang>.json` of a built package, else the source
 * `locales/<lang>.po`. Empty for English and without a file; a broken file
 * throws.
 */
export declare function loadPackageCatalog(directory: string, locale: string): Catalog;
/**
 * Each line of `text` cut to `max` columns, ending with `…` when cut, as
 * the player cuts an OSD panel's lines (`OsdPanelOptions.maxColumns`; the
 * player measures with Unicode's East Asian Width, this is close to it).
 */
export declare function fitColumns(text: string, max: number): string;
export {};
