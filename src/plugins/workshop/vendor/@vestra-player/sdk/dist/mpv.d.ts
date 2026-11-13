/**
 * The mpv pass-through's rules, as data: what `plugin.mpv` may not send, by
 * category, and its limits. The player applies the same table (its
 * agreement test compares them); the SDK refuses early with it and
 * `TestHost` models it.
 *
 * The channel is the SDK's contract; the command and property names and
 * what they do are mpv's (https://mpv.io/manual/stable/#list-of-input-commands,
 * https://mpv.io/manual/stable/#properties, https://mpv.io/manual/stable/#options).
 */
/**
 * Why `plugin.mpv` refuses a command or a property: the boundary it would
 * cross. A key of `MPV_RULES.categories`.
 *
 * @internal
 */
export type MpvRefusalCategory = "process" | "code" | "clients" | "files" | "quit" | "input" | "clipboard" | "managed" | "filters";
/**
 * The limits of `plugin.mpv`, the same values as the player's (its
 * agreement test checks that they agree). Internal: exported from
 * `@vestra-player/sdk/runtime` only; [mpv](../docs/reference/mpv.md) shows
 * them.
 *
 * @internal
 */
export declare const MPV_LIMITS: Readonly<{
    /** Elements of one command, its name included (32). */
    args: 32;
    /** Characters of a command or property name (128). */
    nameChars: 128;
    /** Characters of a filter label, after the `@` (64). */
    labelChars: 64;
    /** Different properties one plugin observes at the same time (64). */
    observed: 64;
    /** A property's changes reach an observer at most this often; the newest value wins (100 ms). */
    observeIntervalMs: 100;
}>;
/**
 * What `plugin.mpv` refuses, as data: the same table as the player's. A
 * command or property not named here goes to mpv as it is.
 *
 * - `categories`: one sentence per category, the end of the refusal's
 *   message.
 * - `commands`: mpv commands by name.
 * - `properties`: mpv properties and options by name (the part before the
 *   first `/`, after an `options/` or `file-local-options/` prefix);
 *   `propertyPrefixes`: by the start of the name, checked after
 *   `properties`. A write is refused; a read too for the categories of
 *   `reads`.
 * - `writers`: commands that write the property their first argument
 *   names (`cycle-values` after an optional `!reverse`); they are checked
 *   as a write of it.
 * - `filterCommands`: the filter chain commands and the operations a
 *   plugin may use, on its own labelled filters only; `filterMessages`:
 *   commands whose first argument is a filter label, which must be the
 *   plugin's own; `reservedLabel`: the start of the player's own labels;
 *   `reservedLabels`: whole labels that are not a plugin's (`all`, which
 *   mpv reads as every filter of the chain).
 *
 * - `prefixes`: mpv's command prefix words (mpv 0.41 `input/cmd.c`,
 *   `cmd_flags`, in its order). mpv takes them off the front of a
 *   command, in the array form too, and runs what follows: a command whose
 *   first element is one is `invalid`, so the name the tables check is
 *   the command mpv runs. Only the first element: later, the same words
 *   are ordinary arguments.
 *
 * A command's name is looked up with `_` read as `-`, as mpv reads it
 * (`cycle_values` is `cycle-values`); a refusal names it as the caller
 * wrote it. A property's name is looked up as written: mpv does not fold
 * it.
 *
 * Internal: exported from `@vestra-player/sdk/runtime` only; the tables of
 * [mpv](../docs/reference/mpv.md) are generated from it.
 *
 * @internal
 */
export declare const MPV_RULES: {
    readonly categories: Readonly<Record<MpvRefusalCategory, string>>;
    readonly commands: Readonly<Record<string, MpvRefusalCategory>>;
    readonly properties: Readonly<Record<string, MpvRefusalCategory>>;
    readonly propertyPrefixes: Readonly<Record<string, MpvRefusalCategory>>;
    readonly reads: readonly MpvRefusalCategory[];
    readonly writers: readonly string[];
    readonly filterCommands: Readonly<Record<string, readonly string[]>>;
    readonly filterMessages: readonly string[];
    readonly reservedLabel: string;
    readonly reservedLabels: readonly string[];
    readonly prefixes: readonly string[];
};
/**
 * What is wrong with a property name, as the player words it (`invalid`);
 * `null` when nothing is.
 *
 * @internal
 */
export declare function mpvNameProblem(name: unknown): string | null;
/**
 * What is wrong with a command's shape, as the player words it (`invalid`);
 * `null` when nothing is. The first element is the command's name, the
 * others its arguments: strings, numbers or booleans.
 *
 * @internal
 */
export declare function mpvCommandProblem(args: unknown): string | null;
/**
 * Why the pass-through refuses a property (`refused`), as the player words
 * it; `null` when it does not.
 *
 * @param name - The property, as given.
 * @param write - A write (`set`); a read is refused only for the
 *   categories of `MPV_RULES.reads`.
 *
 * @internal
 */
export declare function mpvPropertyRefusal(name: string, write: boolean): string | null;
/**
 * Why the pass-through refuses a command (`refused`), as the player words
 * it; `null` when it does not. The command's shape is checked first
 * ({@link mpvCommandProblem}).
 *
 * What only the player can tell is left to it: whether a label belongs to
 * another plugin, and whether a filter value holds more than the one
 * labelled filter (it compares the chain before and after).
 *
 * @internal
 */
export declare function mpvCommandRefusal(args: readonly (string | number | boolean)[]): string | null;
/**
 * The label a filter command adds or removes, for the owner's book:
 * `["vf", "add", "@smooth:…"]` → `{ stage: "vf", operation: "add", label:
 * "smooth" }`; `null` for any other command.
 *
 * @internal
 */
export declare function mpvFilterChange(args: readonly (string | number | boolean)[]): {
    stage: string;
    operation: string;
    label: string;
} | null;
