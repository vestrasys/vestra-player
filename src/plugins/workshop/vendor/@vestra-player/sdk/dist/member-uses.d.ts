/**
 * Where a plugin's TypeScript uses SDK members: the type-aware resolution
 * the capability scan (`capability-scan.ts`) reads a plugin's code with. A
 * use is a property access, an element access with a literal name or a
 * destructured property whose symbol is the SDK's declaration of a member
 * asked for, however the object was reached (`plugin.player.seek`,
 * `const { player } = plugin; player.seek`, a parameter typed `Player`, a
 * `Panel` from `panels.mount`). Not detected: access with a computed name
 * (`player[name]`), values typed `any` or `unknown`, plain JavaScript
 * without types.
 *
 * Requests sent by name (`plugin.wire.request`) are found too, with the
 * method when its type fixes it to one string.
 *
 * The TypeScript module is passed in, never imported: the CLI uses the
 * SDK's own compiler.
 *
 * @internal
 */
import type TS from "typescript";
/** @internal The module plugins import the API types from. */
export declare const SDK_MODULE = "@vestra-player/sdk";
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
 * (`Interface.member`), and every request sent by name (`Wire.request`),
 * in file and position order. It reads no manifest (the capability scan,
 * `capability-scan.ts`, gives it its lists).
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
