import type TS from "typescript";
/** A translatable text found in the sources. */
export interface Message {
    context?: string;
    id: string;
    plural?: string;
    /** `#.` notes (from `TRANSLATORS:` comments). */
    notes: string[];
    /** `path:line` (or `manifest.json`), relative to the plugin, sorted. */
    references: string[];
}
/** Something extraction or the heuristics want the author to look at. */
export interface Finding {
    /** Relative path, forward slashes. */
    file: string;
    line: number;
    kind: "dynamic" | "composed" | "not-english" | "slint-text";
    message: string;
}
export interface ExtractOptions {
    /** The plugin directory. */
    directory: string;
    /** The plugin's `manifest.json` (display fields are extracted). */
    manifest: Record<string, unknown>;
    /** The plugin's `.slint` files, relative to `directory`. */
    slint: readonly string[];
    /** The plugin's production `.ts` / `.mts` files, relative to `directory`. */
    typescript: readonly string[];
    /** Compiler options for a program over those files (as `check` uses). */
    compilerOptions?: TS.CompilerOptions;
}
export declare const pluralForms: (language: string) => "nplurals=1; plural=0;" | "nplurals=2; plural=(n != 1);";
/** Find every translatable text of a plugin, in reference order. */
export declare function extract(options: ExtractOptions): {
    messages: Message[];
    findings: Finding[];
};
/** `locales/messages.pot` for these messages. */
export declare function template(manifest: Record<string, unknown>, messages: readonly Message[]): string;
/** The header of a new `<lang>.po`. */
export declare const newHeader: (language: string) => string;
/**
 * Merge the template into a language's file: translations and fuzzy flags
 * of present messages stay, new ones come untranslated, notes and
 * references follow the template, vanished translated messages become
 * obsolete (`#~`, at the end) and come back with their translation.
 */
export declare function merge(existing: string | null, language: string, messages: readonly Message[]): {
    text: string;
    total: number;
    translated: number;
    added: number;
    obsolete: number;
};
/** Problems found by `check` in one plugin's catalogs, as text lines. */
export declare function checkCatalogs(directory: string, messages: readonly Message[], findings: readonly Finding[]): {
    warnings: string[];
    errors: string[];
};
/**
 * Compile every `locales/<lang>.po` of a player language to
 * `<out>/locales/<lang>.json`. Returns the warnings (files the player never
 * reads); throws on a `.po` that does not parse.
 */
export declare function compileLocales(directory: string, out: string): string[];
/** The relative path of a file from the plugin directory, forward slashes. */
export declare const relativePath: (directory: string, file: string) => string;
