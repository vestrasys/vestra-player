import type { ScopeImpl } from "./scope.ts";
import type { Clipboard, Host, Power, Rendering, Secrets, Storage, System, Tools, MotionEngine, VideoMotion, VideoUpscaling } from "./types.ts";
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
 * @example Remember a position per URL without breaking the limits
 * The plugin keeps the last position of each URL for this player session
 * and skips a URL too long to be a key. Keys are counted in UTF-8 bytes, so
 * the length is measured with `Buffer.byteLength`, not `length`; a `set`
 * that would take the plugin's entries over 1 MiB rejects with `limit` and
 * changes nothing, so the plugin clears the old entries and goes on.
 * ```ts
 * import type { Plugin, PluginError } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare const url: string;
 * declare const position: number;
 * // ---cut---
 * import { SESSION_STORAGE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const session = plugin.storage.session;
 * if (Buffer.byteLength(url) <= SESSION_STORAGE_LIMITS.keyBytes) {
 *   try {
 *     await session.set(url, position);
 *   } catch (error) {
 *     if ((error as PluginError).code !== "limit") throw error;
 *     // Full: start over rather than lose the newest position.
 *     for (const key of await session.keys()) await session.delete(key);
 *     await session.set(url, position);
 *   }
 * }
 * ```
 *
 * @since 1.5.25
 *
 * @category Testing
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
 * @example Store an API token only when it fits
 * The plugin keeps a token the user pasted, or tells them it is empty or
 * too long. Values are 1–1024 bytes counted in UTF-8, so the check uses
 * `Buffer.byteLength`; `secrets.set` would refuse other values with
 * `invalid`. The store itself can still refuse (`unsupported` without a
 * system store, `unavailable` when it cannot be reached, `failed` when the
 * user denies access): only
 * those codes get the "could not be saved" message, anything else (an
 * invalid name) is a bug and is thrown on.
 * ```ts
 * import type { Plugin, PluginError } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare const token: string;
 * // ---cut---
 * import { SECRET_LIMITS } from "@vestra-player/sdk/testing";
 *
 * if (!token || Buffer.byteLength(token) > SECRET_LIMITS.valueBytes) {
 *   await plugin.osd.show("Paste a token of at most 1024 bytes.");
 * } else {
 *   try {
 *     await plugin.secrets.set("api-token", token);
 *   } catch (error) {
 *     const { code } = error as PluginError;
 *     if (!["unsupported", "unavailable", "failed"].includes(code)) throw error;
 *     // No keychain or access denied: ask again next time.
 *     await plugin.osd.show("The token could not be saved.");
 *   }
 * }
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
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
export declare const rendering: (scope: ScopeImpl) => Rendering;
/**
 * `plugin.video.upscaling`: the player
 * answers once its GPU check ran (the first call starts it).
 */
export declare const videoUpscaling: (scope: ScopeImpl) => VideoUpscaling;
/**
 * `plugin.video.motion(engine)`: both requests
 * name the engine; the event names it too and leaves out each plugin's own
 * `mode`, which the SDK keeps from the answers per engine (only this
 * plugin's `set` changes it).
 */
export declare const videoMotion: (scope: ScopeImpl, engine: MotionEngine) => VideoMotion;
