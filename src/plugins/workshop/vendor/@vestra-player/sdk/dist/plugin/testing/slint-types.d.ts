import type { Json } from "../../wire.ts";
import type { TestHost } from "./index.ts";
/** The package directory of the plugin a host runs (set by `startPlugin`). */
export declare const packageDirectories: WeakMap<TestHost, string>;
/** A declared type, resolved. `any`: not known here, every value passes. */
export type SlintType = {
    k: "number" | "string" | "bool" | "image" | "brush" | "enum" | "other";
} | {
    k: "array";
    of: SlintType;
} | {
    k: "struct";
    fields: Map<string, SlintType>;
} | {
    k: "any";
};
type Visibility = "in" | "out" | "in-out" | "private";
/**
 * Slint's `normalize_identifier`: `_` is `-` except as the first
 * character, where `-` is `_`.
 */
export declare const normalize: (name: string) => string;
/** A property as declared: its visibility, type and default rows. */
export interface Declared {
    visibility: Visibility;
    type: SlintType;
    rows: boolean;
    /**
     * The value it holds before any write, as the player's `to_json` gives
     * it, when the test host can tell (`undefined`: it cannot).
     */
    initial: Json | undefined;
}
/**
 * The writable surface of a view: its properties by normalized name.
 * `open`: names not listed may still exist (the component inherits a
 * component this parser does not look into), so they are not refused.
 */
export interface ViewTypes {
    properties: Map<string, Declared>;
    open: boolean;
}
/**
 * The properties of the view `file` (relative to `directory`) shows: its
 * one exported component (a window), or the exported `global` (a panel or
 * settings page). `null` when the file cannot be read or the view cannot
 * be found: then nothing is checked. Kept while none of the files it was
 * read from (the view's and those it imports) changed.
 */
export declare function viewTypes(directory: string, file: string, global?: string): ViewTypes | null;
/** Why the player refuses a write: no such property, an out property, or the type. */
export interface Refusal {
    why: "missing" | "readonly" | "type";
    /** The player's words (`ui_values::refusal`), the property named first. */
    message: string;
}
/**
 * The player's refusal of `value` for `property` of a view of `types`, or
 * `null` when it takes it. `current`: what the property holds (the last
 * value it took; none: its default), which the words of a type refusal
 * describe the declaration by, as the player's do.
 */
export declare function refusal(types: ViewTypes, property: string, value: Json, current: Json | undefined, options?: {
    reconciles?: boolean;
}): Refusal | null;
/**
 * The player's conversion refuses a struct field name not in Slint's
 * canonical spelling (`ui_values`: no `-` first, no `_` after the first
 * character), whatever the declaration says: the first such name in the
 * order the player meets them (fields by name, a field's value before its
 * name), in its words; `null` when there is none. Image references and
 * Markdown documents are not structs.
 */
export declare function spellingProblem(value: Json): string | null;
export {};
