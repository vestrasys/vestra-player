/** The player feature that takes row patches. */
export declare const PATCH_FEATURE = "ui.patch";
/** The most operations one patch carries (the player's `MAX_PATCH_OPS`). */
export declare const PATCH_OPS = 1024;
/**
 * Fields that identify a row, in the order they are tried. A field is used
 * when every row has it, as a string or a number, and no two rows share it.
 */
export declare const KEY_FIELDS: readonly string[];
export type RowKey = string | number;
/** One planned operation; `set` and `insert` name rows of the new array. */
export type PlannedOp = {
    op: "set";
    at: number;
    row: number;
} | {
    op: "insert";
    at: number;
    rows: number[];
} | {
    op: "remove";
    at: number;
    count: number;
} | {
    op: "move";
    from: number;
    count: number;
    to: number;
};
/** What one row counts toward the value limits (as a row of an array). */
export interface RowCount {
    values: number;
    text: number;
    /** Over a per-row limit (a string, the depth): the whole value is sent. */
    over: boolean;
}
/** An array as sent: each row's JSON, its key field and keys, its counts. */
export interface Rows {
    enc: string[];
    field: string | null;
    keys: RowKey[] | null;
    /** Filled in as patches need them. */
    counts: (RowCount | undefined)[];
}
/**
 * The field that identifies `rows`, and each row's key; `null` when no
 * field of {@link KEY_FIELDS} does (rows are then matched by index).
 */
export declare function keyOf(rows: readonly unknown[]): {
    field: string;
    keys: RowKey[];
} | null;
/**
 * What `row` counts toward the value limits as a row of an array (depth
 * 1), like `valueLimitProblem` on the row as sent: every value one, an
 * image one, a document one and its text, strings and field names their
 * UTF-8 bytes. Any other object with `toJSON` counts as what it returns.
 */
export declare function countRow(row: unknown): RowCount;
/** The new array is within the value limits (counted per row). */
export declare function withinLimits(counts: readonly RowCount[]): boolean;
/**
 * The operations that turn the old rows into the new ones (`null`: more
 * than {@link PATCH_OPS} would be needed), and for each new row the old row
 * whose JSON it keeps (`-1`: a new or changed row). Keyed when both arrays
 * have keys from the same field; otherwise by index.
 */
export declare function diff(old: {
    enc: readonly string[];
    keys: readonly RowKey[] | null;
}, next: {
    enc: readonly string[];
    keys: readonly RowKey[] | null;
}): {
    ops: PlannedOp[];
    from: Int32Array;
} | null;
/**
 * Apply planned operations to a copy of `old` (the rows' JSON), taking new
 * rows from `next`: the check that a diff produces exactly the new rows.
 */
export declare function replay(old: readonly string[], ops: readonly PlannedOp[], next: readonly string[]): string[];
/** Apply wire operations to an array (the TestHost's copy of a value). */
export declare function applyOps<T>(rows: T[], ops: readonly ({
    op: "set";
    at: number;
    row: T;
} | {
    op: "insert";
    at: number;
    rows: T[];
} | {
    op: "remove";
    at: number;
    count: number;
} | {
    op: "move";
    from: number;
    count: number;
    to: number;
})[]): T[];
