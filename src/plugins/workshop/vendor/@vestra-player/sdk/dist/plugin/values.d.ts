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
 * They allow a whole list: a plugin binds all of its rows (a playlist of 50,000
 * entries fits) and the view draws them with a Slint `ListView`, which builds
 * only the rows on screen. The SDK sends a changed array as row patches, and an
 * array larger than one request (about 512 KiB of rows) as its first rows
 * followed by `insert` patches of the rest; the player converts large writes
 * off its interface thread.
 *
 * In a row, `null` where the view declares an `image` shows no picture: rows
 * without a thumbnail need no placeholder image.
 *
 * @example Bind a whole list
 * The view draws every row with a `ListView`; the SDK sends what changed.
 *
 * ```ts
 * declare const entries: { id: string; title: string; duration: string }[];
 * declare const list: View;
 * // ---cut---
 * list.bind(() => ({ rows: entries.map((e) => ({ ...e, selected: false })) }));
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
     * Values in all, counting every array, object, string, number, boolean and
     * null, the value itself included; a Markdown document is one (1,048,576).
     */
    readonly values: number;
    /** Rows of one array (100,000). */
    readonly arrayRows: 100000;
    /** UTF-8 bytes of one string or object field name (64 KiB). */
    readonly stringBytes: number;
    /**
     * UTF-8 bytes of all strings and field names together, the text of the
     * value's Markdown documents included (32 MiB).
     */
    readonly textBytes: number;
};
/**
 * The player's runtime image limits (`src/host/src/ui/ui_image.rs`): a file's
 * bytes and sides, the decoded pixels each plugin caches, and the decoded
 * pixels the nested images of one view may show.
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
    /** The decoded pixel bytes the nested images of one view may show. */
    viewBytes: number;
}>;
/**
 * Why the player would refuse `value` (in wire form) as a property value,
 * or `null`. The same checks, order and messages as the player's
 * conversion (`src/host/src/ui/ui_values.rs`). An image reference as the whole
 * value is not counted (the player loads it instead).
 */
export declare function valueLimitProblem(value: Json): string | null;
