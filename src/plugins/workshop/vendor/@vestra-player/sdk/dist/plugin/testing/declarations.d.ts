/**
 * The player's checks of a command declaration (capabilities/commands.rs,
 * commands.rs), with its codes and messages.
 *
 * Part of `TestHost` (`./index.ts`): functions over the host and its private
 * state (`Internals`, `./internals.ts`) that the class delegates to.
 * Internal to the SDK; not exported from `@vestra-player/sdk/testing`.
 */
import { HostError } from "../../wire.ts";
/**
 * The player's refusal of a declaration (commands.rs `CommandError`): code
 * `invalid`, `conflict` or `limit`, its message prefixed as the player's.
 */
export declare const commandInvalid: (message: string) => HostError;
export declare const commandConflict: (message: string) => HostError;
/**
 * The player's check of a command text (commands.rs `text`): 1 to `limit`
 * UTF-8 bytes, no control character (`char::is_control`: C0, DEL and C1)
 * and no whitespace at either end (`str::trim`: Unicode White_Space).
 */
export declare function commandText(value: string, label: string, limit: number): void;
/**
 * A menu text (title or path segment) as the player keys it: a string as
 * declared, a text value by its English rendering (capabilities/commands.rs
 * `menu_text`).
 */
export declare function menuTextKey(value: unknown): string;
/**
 * The declaration checks of the player's `Registry::register`, in its
 * order, on the keyed title and path (`@<id>` already replaced).
 */
export declare function checkDeclaration(id: string, title: string, path: readonly string[], declaration: {
    group?: unknown;
    order?: unknown;
    bindings?: unknown;
    home?: unknown;
}, category?: string | null): void;
