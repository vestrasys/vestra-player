/**
 * Row patches of array properties (player 0.6.0, feature `ui.patch`): the
 * diff between the rows last sent and the rows bound now, as `set`,
 * `insert`, `remove` and `move` operations by index. Internal; plugins see
 * none of it (the player's doc/architecture/large-lists.md §2).
 */
import { UI_VALUE_LIMITS } from "./values.js";
/** The player feature that takes row patches. */
export const PATCH_FEATURE = "ui.patch";
/** The most operations one patch carries (the player's `MAX_PATCH_OPS`). */
export const PATCH_OPS = 1024;
/**
 * Fields that identify a row, in the order they are tried. A field is used
 * when every row has it, as a string or a number, and no two rows share it.
 */
export const KEY_FIELDS = ["key", "id", "rowId", "row-id"];
const plain = (value) => {
    if (value === null || typeof value !== "object" || Array.isArray(value))
        return false;
    const proto = Object.getPrototypeOf(value);
    return proto === Object.prototype || proto === null;
};
/**
 * The field that identifies `rows`, and each row's key; `null` when no
 * field of {@link KEY_FIELDS} does (rows are then matched by index).
 */
export function keyOf(rows) {
    const first = rows[0];
    if (!plain(first))
        return null;
    for (const field of KEY_FIELDS) {
        if (!(field in first))
            continue;
        const keys = [];
        const seen = new Set();
        let ok = true;
        for (let i = 0; i < rows.length; i++) {
            const row = rows[i];
            const key = plain(row) ? row[field] : undefined;
            if ((typeof key !== "string" && typeof key !== "number") ||
                seen.has(key)) {
                ok = false;
                break;
            }
            seen.add(key);
            keys.push(key);
        }
        if (ok)
            return { field, keys };
    }
    return null;
}
const bytes = (text) => Buffer.byteLength(text, "utf8");
/**
 * `value` as `JSON.stringify` writes it: an object's `toJSON` result
 * (once), `undefined` for what it leaves out (a function, a symbol).
 */
function jsonOf(value) {
    if (value !== null &&
        typeof value === "object" &&
        !("__vestraImage" in value) &&
        !("__vestraMarkdown" in value) &&
        typeof value.toJSON === "function")
        return value.toJSON();
    return typeof value === "function" || typeof value === "symbol"
        ? undefined
        : value;
}
/**
 * What `row` counts toward the value limits as a row of an array (depth
 * 1), like `valueLimitProblem` on the row as sent: every value one, an
 * image one, a document one and its text, strings and field names their
 * UTF-8 bytes. Any other object with `toJSON` counts as what it returns.
 */
export function countRow(row) {
    const count = { values: 0, text: 0, over: false };
    const L = UI_VALUE_LIMITS;
    const text = (s) => {
        const n = bytes(s);
        if (n > L.stringBytes)
            count.over = true;
        count.text += n;
    };
    const walk = (value, depth) => {
        if (count.over)
            return;
        if (depth > L.depth) {
            count.over = true;
            return;
        }
        count.values += 1;
        if (value !== null && typeof value === "object") {
            if ("__vestraImage" in value)
                return;
            if ("__vestraMarkdown" in value) {
                count.text += bytes(value.text);
                return;
            }
        }
        if (typeof value === "string")
            return text(value);
        if (Array.isArray(value)) {
            if (value.length > L.arrayRows)
                count.over = true;
            // Array.from: a hole is `null`, as `JSON.stringify` writes it.
            for (const item of Array.from(value))
                walk(jsonOf(item) ?? null, depth + 1);
            return;
        }
        if (value !== null && typeof value === "object")
            for (const [name, raw] of Object.entries(value)) {
                const item = jsonOf(raw);
                if (item === undefined)
                    continue;
                text(name);
                walk(item, depth + 1);
            }
    };
    walk(jsonOf(row) ?? null, 1);
    return count;
}
/** The new array is within the value limits (counted per row). */
export function withinLimits(counts) {
    const L = UI_VALUE_LIMITS;
    if (counts.length > L.arrayRows)
        return false;
    let values = 1;
    let text = 0;
    for (const count of counts) {
        if (count.over)
            return false;
        values += count.values;
        text += count.text;
    }
    return values <= L.values && text <= L.textBytes;
}
/** The longest increasing subsequence of `values`, as a set of indices. */
function increasing(values) {
    const tails = [];
    const before = new Int32Array(values.length).fill(-1);
    for (let i = 0; i < values.length; i++) {
        let lo = 0;
        let hi = tails.length;
        while (lo < hi) {
            const mid = (lo + hi) >> 1;
            if (values[tails[mid]] < values[i])
                lo = mid + 1;
            else
                hi = mid;
        }
        if (lo > 0)
            before[i] = tails[lo - 1];
        tails[lo] = i;
    }
    const out = new Set();
    for (let i = tails.at(-1) ?? -1; i >= 0; i = before[i])
        out.add(i);
    return out;
}
/**
 * The operations that turn the old rows into the new ones (`null`: more
 * than {@link PATCH_OPS} would be needed), and for each new row the old row
 * whose JSON it keeps (`-1`: a new or changed row). Keyed when both arrays
 * have keys from the same field; otherwise by index.
 */
export function diff(old, next) {
    const from = new Int32Array(next.enc.length).fill(-1);
    const ops = old.keys && next.keys
        ? keyed(old.enc, old.keys, next.enc, next.keys, from)
        : byIndex(old.enc, next.enc, from);
    return ops && ops.length <= PATCH_OPS ? { ops, from } : null;
}
function byIndex(old, next, from) {
    const min = Math.min(old.length, next.length);
    let p = 0;
    while (p < min && old[p] === next[p])
        from[p] = p++;
    let s = 0;
    while (s < min - p && old[old.length - 1 - s] === next[next.length - 1 - s]) {
        from[next.length - 1 - s] = old.length - 1 - s;
        s++;
    }
    const ops = [];
    const middle = Math.min(old.length, next.length) - p - s;
    for (let i = p; i < p + middle; i++)
        if (old[i] === next[i])
            from[i] = i;
        else
            ops.push({ op: "set", at: i, row: i });
    const at = p + middle;
    if (next.length > old.length)
        ops.push({
            op: "insert",
            at,
            rows: Array.from({ length: next.length - old.length }, (_, k) => at + k),
        });
    else if (old.length > next.length)
        ops.push({ op: "remove", at, count: old.length - next.length });
    return ops;
}
function keyed(oldEnc, oldKeys, newEnc, newKeys, from) {
    const sets = [];
    const keep = (j, i) => {
        if (oldEnc[i] === newEnc[j])
            from[j] = i;
        else
            sets.push({ op: "set", at: j, row: j });
    };
    const min = Math.min(oldKeys.length, newKeys.length);
    let p = 0;
    while (p < min && oldKeys[p] === newKeys[p])
        keep(p, p++);
    let s = 0;
    while (s < min - p &&
        oldKeys[oldKeys.length - 1 - s] === newKeys[newKeys.length - 1 - s]) {
        keep(newKeys.length - 1 - s, oldKeys.length - 1 - s);
        s++;
    }
    const oldEnd = oldKeys.length - s;
    const newEnd = newKeys.length - s;
    // The new middle's keys, by position (relative to p).
    const target = new Map();
    for (let j = p; j < newEnd; j++)
        target.set(newKeys[j], j - p);
    const ops = [];
    // Removes, from the end so that indices stay valid; runs merged.
    const kept = [];
    const oldAt = new Map();
    for (let i = oldEnd - 1; i >= p; i--) {
        if (target.has(oldKeys[i]))
            continue;
        const last = ops.at(-1);
        if (last?.op === "remove" && last.at === i + 1) {
            last.at = i;
            last.count++;
        }
        else
            ops.push({ op: "remove", at: i, count: 1 });
        if (ops.length > PATCH_OPS)
            return null;
    }
    for (let i = p; i < oldEnd; i++)
        if (target.has(oldKeys[i])) {
            oldAt.set(oldKeys[i], i);
            kept.push(oldKeys[i]);
        }
    // Moves: every kept row outside a longest increasing run of target
    // positions goes right after the kept row before it in the new order.
    const positions = kept.map((key) => target.get(key));
    const stay = increasing(positions);
    if (kept.length - stay.size + ops.length > PATCH_OPS)
        return null;
    const stays = new Set();
    for (const index of stay)
        stays.add(kept[index]);
    const current = kept.slice();
    let previous;
    for (let j = p; j < newEnd; j++) {
        const key = newKeys[j];
        if (!oldAt.has(key))
            continue;
        if (!stays.has(key)) {
            const at = current.indexOf(key);
            current.splice(at, 1);
            const to = previous === undefined ? 0 : current.indexOf(previous) + 1;
            current.splice(to, 0, key);
            if (at !== to)
                ops.push({ op: "move", from: p + at, count: 1, to: p + to });
        }
        previous = key;
    }
    // Inserts, from the top; runs merged.
    for (let j = p; j < newEnd; j++) {
        const key = newKeys[j];
        if (oldAt.has(key)) {
            keep(j, oldAt.get(key));
            continue;
        }
        const last = ops.at(-1);
        if (last?.op === "insert" && last.at + last.rows.length === j)
            last.rows.push(j);
        else
            ops.push({ op: "insert", at: j, rows: [j] });
        if (ops.length > PATCH_OPS)
            return null;
    }
    sets.sort((a, b) => a.at - b.at);
    for (const set of sets) {
        ops.push(set);
        if (ops.length > PATCH_OPS)
            return null;
    }
    return ops;
}
/**
 * Apply planned operations to a copy of `old` (the rows' JSON), taking new
 * rows from `next`: the check that a diff produces exactly the new rows.
 */
export function replay(old, ops, next) {
    const rows = old.slice();
    for (const op of ops)
        switch (op.op) {
            case "set":
                rows[op.at] = next[op.row];
                break;
            case "insert":
                rows.splice(op.at, 0, ...op.rows.map((j) => next[j]));
                break;
            case "remove":
                rows.splice(op.at, op.count);
                break;
            case "move":
                rows.splice(op.to, 0, ...rows.splice(op.from, op.count));
                break;
        }
    return rows;
}
/** Apply wire operations to an array (the TestHost's copy of a value). */
export function applyOps(rows, ops) {
    const out = rows.slice();
    for (const op of ops)
        switch (op.op) {
            case "set":
                out[op.at] = op.row;
                break;
            case "insert":
                out.splice(op.at, 0, ...op.rows);
                break;
            case "remove":
                out.splice(op.at, op.count);
                break;
            case "move":
                out.splice(op.to, 0, ...out.splice(op.from, op.count));
                break;
        }
    return out;
}
