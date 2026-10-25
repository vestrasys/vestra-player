/** The player's limits on one property value, for {@link TestHost}. */
import type { Json } from "../wire.ts";
/**
 * The player's limits on one property value (a `PropValue` as written
 * by `View.set` or `View.bind`, or an initial property of a
 * window, panel or settings page). The player's tests check that these
 * agree with the player; {@link TestHost} enforces them the same way.
 *
 * A value over a limit is refused whole with code `limit` (the property
 * keeps what it showed) and reported to `Scope.onError`: a write by
 * `View.set` or `View.bind` as described there. As an initial property it
 * makes `Window.show` reject; a panel or settings page opens without it and
 * the failure goes to the plugin's `onError`.
 *
 * They bound the work one write costs the player's interface thread; they
 * are not meant as list sizes to aim for. A list that can grow without
 * bound (a playlist, a history) should send the rows near the view and say
 * how many there are, not the whole list.
 *
 * @example
 * ```ts
 * import { UI_VALUE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * // 8 fields per row: 9 values (the row and its fields).
 * const window = Math.floor(UI_VALUE_LIMITS.values / 9) - 1;
 * ```
 *
 * @since 1.5.10
 */
export declare const UI_VALUE_LIMITS: {
    /** Nesting depth; the value itself is depth 0 (8). */
    readonly depth: 8;
    /**
     * Values in all, counting every array, object, string, number, boolean
     * and null, the value itself included; a Markdown document is one
     * (16384).
     */
    readonly values: 16384;
    /** Rows of one array (4096). */
    readonly arrayRows: 4096;
    /** UTF-8 bytes of one string or object field name (64 KiB). */
    readonly stringBytes: number;
    /**
     * UTF-8 bytes of all strings and field names together, the text of the
     * value's Markdown documents included (1 MiB).
     */
    readonly textBytes: number;
};
/**
 * Why the player would refuse `value` (in wire form) as a property value,
 * or `null`. The same checks, order and messages as the player's
 * conversion (`host/src/ui_values.rs`). An image reference as the whole
 * value is not counted (the player loads it instead).
 */
export declare function valueLimitProblem(value: Json): string | null;
