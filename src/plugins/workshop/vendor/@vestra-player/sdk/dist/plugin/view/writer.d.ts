import type { Json, PropertyWrite } from "../../wire.ts";
import type { PropValue, Props } from "../types.ts";
/** Slint names are kebab-case; plugin code may use camelCase. */
export declare const kebab: (name: string) => string;
export declare const encode: (props: Props) => {
    [k: string]: PropValue;
};
/** Where a {@link ViewWriter}'s writes go, and what it asks of its view. */
export interface WriterTarget {
    /** The view shows its properties: writes may be sent. */
    live(): boolean;
    /** Send one property write to the player. */
    write(property: string, write: PropertyWrite): Promise<void>;
    /** Report a failure (the plugin's error handler and the player's log). */
    report(error: unknown): void;
    /** The player takes row patches (and `seq` on whole writes). */
    patches(): boolean;
}
/**
 * One view's properties: their values now, what the player shows as far as
 * this side knows, and the writes that bring the one to the other.
 */
export declare class ViewWriter {
    #private;
    constructor(target: WriterTarget);
    /**
     * Initial properties in wire form. A released image is left out (the
     * property keeps its default) and reported, like a failed write. With a
     * player that takes row patches, the arrays of one request carry at most
     * {@link CHUNK_BYTES} of rows together (SDK 1.5.46): an array over that
     * goes with its first rows, and the first flush sends the rest as patches.
     * An array over the value limits is refused here, as the player would
     * refuse it whole: left out and reported (a panel, a settings page), or,
     * with `refuse` (a window's `show`), thrown.
     */
    wired(values: Record<string, PropValue>, refuse?: boolean): Record<string, Json>;
    /** `View.set`: each value is sent once even if equal to what was sent. */
    set(props: Props): void;
    /** A bound value (Slint name): sent by the next flush if it changed. */
    assign(key: string, value: PropValue): void;
    /** Every property's value now, by Slint name. */
    values(): Record<string, PropValue>;
    /** The host already shows these values (e.g. sent as initial properties). */
    shown(values: Record<string, Json>): void;
    /**
     * Send what changed. A bound value is sent when it differs from the last one
     * sent; a `set()` value is always sent once. Nothing is resent merely
     * because a callback arrived, so text the user typed into an in-out
     * property survives other callbacks (call `set()` to pull a control back).
     * A failed bound value is not repeated until it changes; an explicit `set()`
     * is, after a back-off (0.25 s doubling to 30 s). Only the first failure of
     * a failing streak is reported, so an error handler that writes a status to
     * a broken view cannot spin.
     */
    flush(): void;
    /** The view closed: no retry after a back-off. */
    stop(): void;
}
