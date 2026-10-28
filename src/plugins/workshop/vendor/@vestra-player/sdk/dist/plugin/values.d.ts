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
 * @example Send a long playlist as a window of rows
 * A list view shows any number of entries while each write stays small:
 * the plugin sends only the rows around the scroll position plus the total,
 * and the view reports where it scrolled to. After the `scrolled` handler
 * returns, the SDK evaluates the `bind` function again and sends the new
 * window. Each row object and each of its fields count as one value toward
 * `values`, and so does the array holding them.
 * ```ts
 * declare const entries: { title: string; duration: number }[];
 * import type { View } from "@vestra-player/sdk";
 * declare const list: View;
 * // ---cut---
 * import { UI_VALUE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * // 200 rows, or fewer if the limits were smaller: 2 fields per row make
 * // 3 values (the row and its fields), plus 1 for the array.
 * const size = Math.min(
 *   200,
 *   UI_VALUE_LIMITS.arrayRows,
 *   Math.floor((UI_VALUE_LIMITS.values - 1) / 3),
 * );
 * let first = 0;
 * list.bind(() => ({
 *   total: entries.length,
 *   rows: entries.slice(first, first + size),
 * }));
 * // The Slint view reports its first visible row; the window follows it.
 * list.on("scrolled", (row: number) => {
 *   first = Math.max(0, row - Math.floor(size / 2));
 * });
 * ```
 *
 * @since 1.5.10
 *
 * @category Testing
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
 * The player's runtime image limits (`src/host/src/ui/ui_image.rs`): a file's
 * bytes and sides, the decoded pixels each plugin caches, and (SDK 1.5.19,
 * player 0.5.1) the decoded pixels the nested images of one view may show.
 *
 * @since 1.5.19
 * @category Testing
 */
export declare const IMAGE_LIMITS: Readonly<{
    /** The largest image file the player reads, in bytes. */
    fileBytes: number;
    /** The longest side, in pixels, the player accepts for an image file. */
    side: 4096;
    /** The decoded pixel bytes the player caches for each plugin's images. */
    pluginBytes: number;
    /** The decoded pixel bytes the nested images of one view may show (player 0.5.1). */
    viewBytes: number;
}>;
/**
 * Why the player would refuse `value` (in wire form) as a property value,
 * or `null`. The same checks, order and messages as the player's
 * conversion (`src/host/src/ui/ui_values.rs`). An image reference as the whole
 * value is not counted (the player loads it instead).
 */
export declare function valueLimitProblem(value: Json): string | null;
