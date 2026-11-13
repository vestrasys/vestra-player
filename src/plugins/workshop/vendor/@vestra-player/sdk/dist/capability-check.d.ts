/**
 * The capability checker: every use of a plugin API member whose capability
 * the plugin's `manifest.json` does not declare. `vestra-plugin check` (and
 * so `build` and `pack`) and the TypeScript language-service plugin
 * (`@vestra-player/sdk/ts-plugin`) both run it; the rule is
 * `capability-rules.ts` (the player's table and the SDK's requests).
 *
 * It is type-aware: a use is a property access, an element access with a
 * literal name or a destructured property whose symbol is the SDK's
 * declaration of a member in the table, however the object was reached
 * (`plugin.player.seek`, `const { player } = plugin; player.seek`, a
 * parameter typed `Player`, a `Panel` from `panels.mount`). Not detected:
 * access with a computed name (`player[name]`), values typed `any` or
 * `unknown`, plain JavaScript without types, members inherited from `View`
 * (their object comes from a call that is checked) and options that need
 * more (a `processed` capture, a file resolver).
 *
 * Members whose capability depends on an argument (`plugin.player.on` and
 * `plugin.wire.on` by event name, `plugin.wire.request` by method name,
 * `ARGUMENT_APIS`) are checked by that argument when its type fixes it to one
 * string (a literal, a constant); otherwise (a computed name, a union of
 * several, the member taken by destructuring or as a value) the call is
 * reported as unchecked ({@link checkCapabilities}), and the player still
 * checks it.
 *
 * The TypeScript module is passed in, never imported: the CLI uses the
 * SDK's own compiler, the editor its TypeScript server's.
 *
 * @internal
 */
import type TS from "typescript";
import type { CapabilityRules } from "./capability-rules.ts";
/** @internal The module plugins import the API types from. */
export declare const SDK_MODULE = "@vestra-player/sdk";
/** @internal The diagnostic code the editor shows (and its quick fix keys on). */
export declare const CAPABILITY_DIAGNOSTIC = 95301;
/**
 * @internal The warning code the editor shows for a call whose capability
 * the checker cannot read (`plugin.wire.request` with a computed method).
 */
export declare const CAPABILITY_UNCHECKED = 95302;
/** @internal One use of a member whose capability is not declared. */
export interface CapabilityFinding {
    /** The source file, as the program names it. */
    readonly file: string;
    /** 1-based line and column of the member's name. */
    readonly line: number;
    readonly column: number;
    /** Offset and length of the member's name in the file. */
    readonly start: number;
    readonly length: number;
    /** The member, `Interface.member` (`Player.seek`). */
    readonly api: string;
    /** The capability to declare. */
    readonly capability: string;
    /** The request that needs it. */
    readonly method: string;
    /** The declared-or-needed capability whose `requires` names it. */
    readonly requiredBy?: string;
    /** One line for people. */
    readonly message: string;
}
/** @internal */
export interface CheckOptions {
    /** The files to check (others in the program are only read for types). */
    readonly files: readonly string[];
    /** The manifest's `capabilities`. */
    readonly capabilities: readonly string[];
    /** Default: the SDK's rules. */
    readonly rules?: CapabilityRules;
}
/**
 * @internal A call whose capability the checker cannot tell: a member of
 * `ARGUMENT_APIS` (`plugin.wire.request`, `player.on`, `wire.on`) whose name
 * argument is not a string the types fix, or a method the player's table does
 * not know. The player still checks it at run time.
 */
export interface UncheckedCall {
    readonly file: string;
    /** 1-based line and column of the member's name. */
    readonly line: number;
    readonly column: number;
    /** Offset and length of the member's name in the file. */
    readonly start: number;
    readonly length: number;
    /** The member, `Interface.member` (`Wire.request`). */
    readonly api: string;
    /** One line for people. */
    readonly message: string;
}
/**
 * Every use, in `options.files`, of a member whose capability the manifest
 * does not declare, in file and position order.
 *
 * @internal
 */
export declare function findUndeclaredCapabilities(ts: typeof TS, program: TS.Program, options: CheckOptions): CapabilityFinding[];
/**
 * The capability check: the uses of members whose capability the manifest does
 * not declare (`findings`, as {@link findUndeclaredCapabilities}), and the
 * calls whose capability depends on a name the checker cannot read
 * (`unchecked`).
 *
 * A call of `plugin.player.on` or `plugin.wire.on` is checked by its event
 * name, one of `plugin.wire.request` by its method name and the string
 * fields of its parameters (`{ event: "…" }`), when the types fix them.
 *
 * @internal
 */
export declare function checkCapabilities(ts: typeof TS, program: TS.Program, options: CheckOptions): {
    findings: CapabilityFinding[];
    unchecked: UncheckedCall[];
};
/**
 * Whether a file belongs to the plugin's production code, given the
 * directory of its `manifest.json`: not in `tests/`, `node_modules/`,
 * `dist/`, `vendor/` or a dot directory (what `vestra-plugin build` leaves
 * out), and not a declaration file.
 *
 * @internal
 */
export declare function isPluginSource(directory: string, file: string): boolean;
/**
 * The edit that adds a capability to a manifest's `capabilities` array, in
 * the array's own layout; `undefined` when the text has no such array.
 *
 * @internal
 */
export declare function addCapabilityEdit(ts: typeof TS, text: string, capability: string): {
    start: number;
    length: number;
    newText: string;
} | undefined;
/** @internal One use of an SDK member, where {@link findMemberUses} found it. */
export interface MemberUse {
    readonly file: string;
    /** 1-based line and column of the member's name. */
    readonly line: number;
    readonly column: number;
    /** The member, as it was asked for (`Clipboard.readText`). */
    readonly api: string;
}
/**
 * @internal A request sent by name (`plugin.wire.request("clipboard.readText")`):
 * the method when the types fix it to one string, else `undefined` (a
 * computed name, a union of several, the member taken as a value).
 */
export interface RequestUse {
    readonly file: string;
    readonly line: number;
    readonly column: number;
    readonly method: string | undefined;
}
/**
 * Every use, in `files`, of the SDK members named by `apis`
 * (`Interface.member`), and every request sent by name
 * (`ARGUMENT_APIS`' `method` members: `Wire.request`), in file and position
 * order. The same type-aware resolution as {@link checkCapabilities}, for
 * any list of members: a use is a property access, an element access with
 * a literal name or a destructured property whose symbol is the SDK's
 * declaration of the member. It reads no manifest and no capability rule
 * (the capability scan, `capability-scan.ts`, gives it its own lists).
 *
 * `resolved` is `false` when the program has no import of
 * `@vestra-player/sdk` that resolves: nothing can be found by type then.
 *
 * @internal
 */
export declare function findMemberUses(ts: typeof TS, program: TS.Program, options: {
    readonly files: readonly string[];
    readonly apis: Iterable<string>;
}): {
    resolved: boolean;
    uses: MemberUse[];
    requests: RequestUse[];
};
