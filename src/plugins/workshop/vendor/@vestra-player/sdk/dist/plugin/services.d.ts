import type { ScopeImpl } from "./scope.ts";
import type { Clipboard, Host, Power, Secrets, Storage, System, Tools } from "./types.ts";
export declare const clipboard: (scope: ScopeImpl) => Clipboard;
export declare const host: (scope: ScopeImpl) => Host;
export declare const power: (scope: ScopeImpl) => Power;
export declare function tools(scope: ScopeImpl): Tools;
/**
 * The player's limits on {@link SessionStorage}. A key outside them fails
 * with code `invalid`; a `set` that would make the plugin's entries larger
 * than `bytes` fails with `limit` and changes nothing. A single `set` must
 * also fit in one message to the player
 * ({@link "@vestra-player/sdk/testing"!MESSAGE_LIMIT | MESSAGE_LIMIT}, 1 MiB
 * with the key and the request around the value). The player's tests check
 * that these agree with the player.
 *
 * @example
 * ```ts
 * import assert from "node:assert/strict";
 * import { SESSION_STORAGE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const key = "https://example.com/a.mkv";
 * assert.ok(Buffer.byteLength(key) <= SESSION_STORAGE_LIMITS.keyBytes);
 * ```
 *
 * @since 1.5.25
 */
export declare const SESSION_STORAGE_LIMITS: Readonly<{
    /** UTF-8 bytes of a key (1024, at least 1; no NUL character). */
    readonly keyBytes: 1024;
    /**
     * Bytes one plugin may hold at the same time: for every key, its UTF-8
     * bytes, its value written as compact JSON (`JSON.stringify(value)`) and
     * `entryBytes`.
     */
    readonly bytes: number;
    /**
     * What each key costs on top of its text and value (128): the player's
     * own bookkeeping, so that many tiny entries stay within `bytes` of memory.
     */
    readonly entryBytes: 128;
}>;
export declare const storage: (scope: ScopeImpl) => Storage;
export declare function system(scope: ScopeImpl): System;
/**
 * The player's limits on {@link Secrets}. A name or value outside them
 * fails with code `invalid`; a new name beyond `names` fails with `limit`.
 * The player's tests check that these agree with the player.
 *
 * @example
 * ```ts
 * import { SECRET_LIMITS } from "@vestra-player/sdk/testing";
 *
 * assert.ok(Buffer.byteLength(token) <= SECRET_LIMITS.valueBytes);
 * ```
 *
 * @since 1.5.14
 */
export declare const SECRET_LIMITS: Readonly<{
    /**
     * Characters of a name (64, at least 1), each one of `a-z`, `0-9`, `.`,
     * `_` and `-`.
     */
    readonly nameChars: 64;
    /** UTF-8 bytes of a value (1024, at least 1; no NUL character). */
    readonly valueBytes: 1024;
    /** Different names one plugin may have stored at the same time (256). */
    readonly names: 256;
}>;
export declare const secrets: (scope: ScopeImpl) => Secrets;
