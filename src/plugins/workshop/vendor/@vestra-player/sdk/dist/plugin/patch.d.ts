/** The player feature that takes row patches. */
export declare const PATCH_FEATURE = "ui.patch";
/** The most operations one patch carries (the player's `MAX_PATCH_OPS`). */
export declare const PATCH_OPS = 1024;
/**
 * The most operations one diff plans (SDK 1.5.46): its patch is sent as
 * several of at most {@link PATCH_OPS} each (`chunkOps`); more is sent whole
 * (in parts, which the player writes at once).
 */
export declare const DIFF_OPS: number;
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
 * than {@link DIFF_OPS} would be needed, or more than {@link PATCH_OPS}
 * moves), and for each new row the old row
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
/**
 * The bytes of rows one write carries at most (SDK 1.5.46, player 0.6.0):
 * half the 1 MiB protocol line, which leaves room for the request around
 * them. A larger whole write is sent as its first rows and `insert`
 * patches of the rest; a larger patch as several patches, each applying on
 * the one before (the player's doc/architecture/large-lists.md §4).
 */
export declare const CHUNK_BYTES: number;
/** The bytes a row adds to a write (its JSON and a separator). */
export declare const rowBytes: (row: unknown) => number;
/**
 * How many of the first rows (by their sizes, `rowBytes`) fit in `budget`
 * bytes; at least one when there are rows (a row over the budget alone is
 * left to the player's limits).
 */
export declare function prefixWithin(sizes: readonly number[], budget: number): number;
/** A wire operation (`set` and `insert` carry rows). */
export type WireOp = {
    op: "set";
    at: number;
    row: unknown;
} | {
    op: "insert";
    at: number;
    rows: unknown[];
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
/**
 * `ops` as consecutive patches of at most {@link CHUNK_BYTES} of rows and
 * {@link PATCH_OPS} operations each, in order: applying them one after the
 * other gives what `ops` give. An `insert` of more rows than fit is split
 * into inserts of consecutive runs.
 */
export declare function chunkOps<T extends WireOp>(ops: readonly T[]): T[][];
/** The bytes a row already in JSON adds to a write (and a separator). */
export declare const jsonBytes: (json: string) => number;
