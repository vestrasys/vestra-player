/**
 * The view writer (L1, a primitive; doc/specs/sdk-refactor.md §2.2 in the
 * player repository): a view's properties as last sent, and the write path
 * from a value to `ui.set` / `ui.setMount` / `settings.set` — encoding,
 * comparison with what was sent, row patches (planned by `patch.ts`),
 * parts of large writes (`more`), the value limits, `stale` fallback and
 * the back-off after a failed write. It knows nothing of update passes or
 * bindings: the binder (`binder.ts`, L2) puts values in, the view
 * (`base.ts`) says where a write goes.
 *
 * Two modes (SDK 1.7.0, refactor Step 2). Until the view's first `bind()`
 * (`track()`), the writer serves `set()` alone: each value is sent whole
 * (limits checked, large arrays in parts with `more`, one write in
 * flight per view), only the properties `set()` named are looked at, and
 * no row-patch base is kept, so nothing is diffed. Once tracked it is the
 * binder's: every value is compared with what was sent and arrays become
 * row patches where one applies; a `set()` then takes the same path, so
 * mixed use stays consistent. Patch planning stays here, used only in the
 * tracked mode: it needs what was sent and the player's base, which are
 * this writer's, and moving it above would split one property's write
 * state across two layers.
 *
 * A third track (SDK 1.7.0, refactor Step 3): properties written directly
 * through `plugin.wire.view(v)` (`direct`, `directRows`). Such a property
 * is the plugin's alone: its `PropertyWrite`s and `RowOp`s go out as
 * given, with only one write in flight per property, a `seq` when the
 * player takes patches, and large writes in parts; nothing is encoded,
 * compared or checked against the limits. One track per property: a
 * property written directly cannot be `set()` or bound, and the other
 * way round (`invalid`), so `#sent` and `#bases` never go stale.
 */
import { HostError } from "../../wire.js";
import { MarkdownHandle } from "../markdown.js";
import { countRow, diff, keyOf, CHUNK_BYTES, chunkOps, prefixWithin, jsonBytes, rowBytes, replay, withinLimits, } from "../patch.js";
import { UI_VALUE_LIMITS, valueLimitProblem } from "../values.js";
import { failure } from "../errors.js";
import { ImageHandle } from "./image.js";
/** Slint names are kebab-case; plugin code may use camelCase. */
export const kebab = (name) => name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
export const encode = (props) => Object.fromEntries(Object.entries(props).map(([k, v]) => [kebab(k), v]));
/**
 * A property value as sent: images and documents in wire form, at any depth
 * (released ones refused wherever they are), like the player.
 */
function wireValue(value) {
    if (value instanceof MarkdownHandle) {
        if (value.released)
            throw failure("invalid", "Markdown document was released with the scope that created it");
        return value.toJSON();
    }
    if (value instanceof ImageHandle) {
        if (value.released)
            throw failure("invalid", `image ${value.path} was released with the scope that created it`);
        return value.toJSON();
    }
    // Array.from: a hole is `null`, as `JSON.stringify` writes it.
    if (Array.isArray(value))
        return Array.from(value, (item) => wireValue(item ?? null));
    if (value !== null && typeof value === "object") {
        const out = {};
        for (const [key, item] of Object.entries(value))
            out[key] = wireValue(item);
        return out;
    }
    return value;
}
/** Scalars compared by value. Non-finite numbers are JSON `null`: compared as JSON. */
const isPrimitive = (value) => value === null ||
    typeof value === "string" ||
    typeof value === "boolean" ||
    (typeof value === "number" && Number.isFinite(value));
/** A row as `JSON.stringify` writes it inside an array. */
const encodeRow = (row) => JSON.stringify(row) ?? "null";
/** What a row's encoding holds: a Markdown document, an image. */
const HAS_MARKDOWN = 1;
const HAS_IMAGE = 2;
const marksOf = (enc) => (enc.includes('"$vestraMarkdown"') ? HAS_MARKDOWN : 0) |
    (enc.includes('"$vestraImage"') ? HAS_IMAGE : 0);
/**
 * Row encodings by row object, shared by every property (a row's JSON does
 * not depend on where it is bound). Only rows that cannot change are kept
 * ({@link settled}): a plugin that mutates its rows in place sees no
 * difference, and one that keeps unchanged rows as the same deep-frozen
 * objects skips encoding them on every pass. Weak: a row the plugin drops
 * goes with its entry.
 */
const settledRows = new WeakMap();
/**
 * Frozen rows seen once. A frozen row is verified and kept on its second
 * pass, so rows frozen afresh on every pass cost one set entry, not a
 * verification each.
 */
const seenRows = new WeakSet();
/** Frozen rows that can still change (a mutable part, a getter): never kept. */
const unsettledRows = new WeakSet();
/**
 * `value` can never change what `JSON.stringify` writes for it: a primitive,
 * an image or document handle (their wire form is fixed when made), or a
 * frozen plain object or array whose own enumerable properties are data
 * properties (no getter, no own `toJSON`) holding such values. Freezing
 * fixes the properties and the prototype for good, so the answer holds as
 * long as the object lives.
 */
function settled(value, depth) {
    if (value === null || typeof value !== "object")
        return typeof value !== "function";
    if (value instanceof ImageHandle || value instanceof MarkdownHandle)
        return true;
    if (depth > UI_VALUE_LIMITS.depth || !Object.isFrozen(value))
        return false;
    const proto = Object.getPrototypeOf(value);
    if (Array.isArray(value)
        ? proto !== Array.prototype
        : proto !== Object.prototype && proto !== null)
        return false;
    // An own `toJSON`, enumerable or not, is called by `JSON.stringify`.
    if (Object.prototype.hasOwnProperty.call(value, "toJSON"))
        return false;
    for (const key of Object.keys(value)) {
        const property = Object.getOwnPropertyDescriptor(value, key);
        if (!property || !("value" in property))
            return false;
        if (!settled(property.value, depth + 1))
            return false;
    }
    return true;
}
/** A row's encoding and marks, from {@link settledRows} when it is kept. */
function rowEncoding(row, cache) {
    if (cache && row !== null && typeof row === "object") {
        // A kept row is frozen: looked up first, `isFrozen` only on a miss.
        const kept = settledRows.get(row);
        if (kept)
            return kept;
    }
    if (cache &&
        row !== null &&
        typeof row === "object" &&
        Object.isFrozen(row)) {
        const enc = encodeRow(row);
        const entry = { enc, marks: marksOf(enc) };
        if (!seenRows.has(row))
            seenRows.add(row);
        else if (!unsettledRows.has(row)) {
            if (settled(row, 1))
                settledRows.set(row, entry);
            else
                unsettledRows.add(row);
        }
        return entry;
    }
    const enc = encodeRow(row);
    return { enc, marks: marksOf(enc) };
}
/** Each row of an array as `JSON.stringify` writes it; a hole is `null`. */
function encodeRows(rows) {
    // A `toJSON` added to the shared prototypes would change any row's JSON:
    // then nothing is taken from (or kept in) the cache.
    const cache = !("toJSON" in Object.prototype) && !("toJSON" in Array.prototype);
    const enc = [];
    const marks = new Uint8Array(rows.length);
    let markdown = false;
    for (let i = 0; i < rows.length; i++) {
        const row = rowEncoding(rows[i], cache);
        enc[i] = row.enc;
        marks[i] = row.marks;
        if (row.marks & HAS_MARKDOWN)
            markdown = true;
    }
    return { enc, marks, markdown, cache };
}
/** A row's limit counts, kept with its encoding when the row cannot change. */
function rowCount(row, cache) {
    const kept = cache && row !== null && typeof row === "object"
        ? settledRows.get(row)
        : undefined;
    if (!kept)
        return countRow(row);
    return (kept.count ??= countRow(row));
}
function sameRows(a, b) {
    if (a.length !== b.length)
        return false;
    for (let i = 0; i < a.length; i++)
        if (a[i] !== b[i])
            return false;
    return true;
}
/** An initial property (wire form) as sent. */
function sentOf(value, patches) {
    if (isPrimitive(value))
        return { kind: "value", value };
    if (Array.isArray(value) && patches) {
        const keyed = keyOf(value);
        return {
            kind: "rows",
            rows: {
                enc: encodeRows(value).enc,
                field: keyed?.field ?? null,
                keys: keyed?.keys ?? null,
                counts: [],
            },
        };
    }
    return { kind: "json", enc: JSON.stringify(value) };
}
/**
 * A write sent in parts stopped because its view closed or showed its
 * initial properties again: not a failure.
 */
class Abandoned extends Error {
}
/** The wire key of a Markdown document: rows holding one are never sent in parts. */ const MARKDOWN_KEY = '"$vestraMarkdown"';
/** A row larger than one request can carry (`limit`). */
const rowTooLarge = (key, bytes) => new HostError("limit", `${key}: a row of ${bytes} bytes is larger than one request takes (${CHUNK_BYTES} bytes)`);
/** Refuse a patch carrying a row larger than one request, before sending it. */
function tooLarge(key, ops) {
    for (const op of ops) {
        const rows = op.op === "set" ? [op.row] : op.op === "insert" ? op.rows : [];
        for (const row of rows) {
            const bytes = rowBytes(row);
            if (bytes > CHUNK_BYTES)
                throw rowTooLarge(key, bytes);
        }
    }
}
/**
 * The row patch from `sent` to `next` (the bound `rows`), in wire form, and
 * the new rows' limit counts; `null` when the whole value is to be sent:
 * nothing to type the rows by (no rows sent), a Markdown document in a row
 * (the player merges documents in place), too many operations, a value over
 * the limits (the player refuses it whole, as before), or no change (an
 * explicit `set()` of the same value).
 */
function rowPatch(sent, next, rows) {
    if (sent.enc.length === 0)
        return null;
    if (next.markdown)
        return null;
    const nextKeyed = sent.field === null ? null : next.keyed();
    const keyed = nextKeyed !== null && nextKeyed.field === sent.field;
    const plan = diff({ enc: sent.enc, keys: keyed ? sent.keys : null }, { enc: next.enc, keys: keyed ? nextKeyed.keys : null });
    if (!plan || plan.ops.length === 0)
        return null;
    // A released image is refused wherever it is, as in a whole write: the
    // rows the patch does not carry are checked too (those holding images).
    for (let j = 0; j < rows.length; j++)
        if (plan.from[j] >= 0 && next.marks[j] & HAS_IMAGE)
            wireValue(rows[j] ?? null);
    const counts = next.enc.map((_, j) => {
        const from = plan.from[j];
        return ((from >= 0 ? sent.counts[from] : undefined) ??
            rowCount(rows[j], next.cache));
    });
    if (!withinLimits(counts))
        return null;
    // The ops must give exactly the new rows (a guard against this diff's bugs).
    if (!sameRows(replay(sent.enc, plan.ops, next.enc), next.enc))
        return null;
    const ops = plan.ops.map((op) => {
        switch (op.op) {
            case "set":
                return { op: "set", at: op.at, row: wireValue(rows[op.row] ?? null) };
            case "insert":
                return {
                    op: "insert",
                    at: op.at,
                    rows: op.rows.map((j) => wireValue(rows[j] ?? null)),
                };
            default:
                return op;
        }
    });
    return { ops, counts };
}
/**
 * How `plugin.wire.view(v)` (`wire-facade.ts`) reaches a view's writer
 * without importing the view classes (which load the binder): a view
 * answers this key with its {@link DirectWrites}, given a scope of the
 * plugin that made it.
 */
export const DIRECT = Symbol("direct writes");
/** The message of a write on the other track of a property. */
export const bothTracks = (property, written) => written
    ? `property \`${property}\` is written directly; bind it or write it, not both`
    : `property \`${property}\` is set or bound; bind it or write it, not both`;
/** Row operations `chunkOps` can split: the four shapes, rows in arrays. */
const wellFormed = (ops) => ops.every((op) => op !== null &&
    typeof op === "object" &&
    (op.op === "set" ||
        op.op === "remove" ||
        op.op === "move" ||
        (op.op === "insert" && Array.isArray(op.rows))));
/**
 * One view's properties: their values now, what the player shows as far as
 * this side knows, and the writes that bring the one to the other.
 */
export class ViewWriter {
    #target;
    #values = new Map();
    /** What the host currently shows, as far as this side knows. */
    #sent = new Map();
    /**
     * Row patches (player feature `ui.patch`): each property's last write the
     * player confirmed, by `seq` (0: the initial properties). A patch applies
     * on it; a property without one is sent whole.
     */
    #bases = new Map();
    /**
     * Patches of a property answered `stale` in a row: after two, the
     * property is sent whole until the view shows its initial properties again.
     */
    #stales = new Map();
    #seq = 0;
    /** Counts the times the view showed its initial properties (`shown`). */
    #generation = 0;
    /** Written by `set()`: sent even if equal to what was sent before. */
    #forced = new Set();
    /** The last value of each property whose write failed; not retried until it changes. */
    #failed = new Map();
    #backoff = 0;
    /** A write failed and none succeeded since: further failures stay quiet. */
    #broken = false;
    #retry;
    #flushing = false;
    /**
     * The view was bound (`track()`): every value is compared with what was
     * sent and arrays are patched. Before, only `set()` values are sent,
     * whole, and no patch base is kept.
     */
    #tracked = false;
    /**
     * Initial arrays `wired` cut to one request's size, by key: how many
     * rows went (until `shown`).
     */
    #cut = new Map();
    /**
     * Not bound: initial arrays the view shows the first rows of, by key,
     * whose next write inserts only the rest (SDK 1.7.0).
     */
    #rest = new Map();
    /** Properties written directly (`plugin.wire.view(v)`, SDK 1.7.0). */
    #direct = new Map();
    /** Direct properties a binding produced: reported once each. */
    #refused = new Set();
    constructor(target) {
        this.#target = target;
    }
    /**
     * The view's first `bind()`: compare and patch from now on. What was
     * sent before is known as whole values, so the first patchable write of
     * an array goes whole and patches follow it.
     */
    track() {
        this.#tracked = true;
        // Bound values go the bound way, compared with what was sent.
        this.#rest.clear();
    }
    /** Row patches apply: the player takes them and the view is bound. */
    get #patching() {
        return this.#tracked && this.#patches;
    }
    /** The player takes row patches (and `seq` on whole writes). */
    get #patches() {
        return this.#target.patches();
    }
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
    wired(values, refuse = false) {
        const out = {};
        let room = CHUNK_BYTES;
        for (const [key, value] of Object.entries(values)) {
            let wire;
            try {
                wire = wireValue(value);
            }
            catch (error) {
                this.#failed.set(key, JSON.stringify(value));
                this.#target.report(error);
                continue;
            }
            if (this.#patches && Array.isArray(wire)) {
                const sizes = wire.map(rowBytes);
                const total = sizes.reduce((a, b) => a + b, 0);
                if (total > room && !JSON.stringify(wire).includes(MARKDOWN_KEY)) {
                    const largest = sizes.reduce((a, b) => Math.max(a, b), 0);
                    const problem = valueLimitProblem(wire);
                    if (problem || largest > CHUNK_BYTES) {
                        const error = problem
                            ? new HostError("limit", `${key}: ${problem}`)
                            : rowTooLarge(key, largest);
                        if (refuse)
                            throw error;
                        this.#failed.set(key, JSON.stringify(value));
                        this.#target.report(error);
                        continue;
                    }
                    const count = room > 0 ? prefixWithin(sizes, room) : 0;
                    wire = wire.slice(0, count);
                    this.#cut.set(key, count);
                    room -= sizes.slice(0, count).reduce((a, b) => a + b, 0);
                }
                else
                    room -= total;
            }
            out[key] = wire;
        }
        return out;
    }
    /**
     * `View.set`: each value is sent once even if equal to what was sent. A
     * property written directly is refused (`invalid`), before anything is
     * kept.
     */
    set(props) {
        const values = Object.entries(encode(props));
        for (const [key] of values)
            if (this.#direct.has(key))
                throw failure("invalid", bothTracks(key, true));
        for (const [key, value] of values) {
            this.#values.set(key, value);
            this.#forced.add(key);
            this.#failed.delete(key); // an explicit write is tried again (after any back-off)
            this.#rest.delete(key); // a new value: no longer the initial rows' rest
        }
        this.flush();
    }
    /**
     * A bound value (Slint name): sent by the next flush if it changed. A
     * property written directly is refused (`invalid`, the first time; the
     * binder reports it) and the value is never kept.
     */
    assign(key, value) {
        if (this.#direct.has(key)) {
            if (this.#refused.has(key))
                return;
            this.#refused.add(key);
            throw failure("invalid", bothTracks(key, true));
        }
        this.#values.set(key, value);
    }
    /**
     * `plugin.wire.view(v).write`: one property's `PropertyWrite`, sent as
     * given after the property's earlier direct writes (a whole value drops
     * those still waiting, which resolve `false`). With a player that takes
     * row patches, a whole value without `seq` gets one, so that row
     * operations can follow it, and an array larger than one request goes in
     * parts (`more`). The property is marked: `set()` and `bind()` refuse it
     * from now on. Rejects with `unavailable` while the view does not show.
     */
    direct(property, write) {
        const key = this.#directKey(property);
        if (write === null ||
            typeof write !== "object" ||
            !("value" in write || "patch" in write))
            throw failure("invalid", "write takes a PropertyWrite: { value } or { patch: { base, seq, ops } }");
        return this.#queueDirect(key, { kind: "write", write }, !("patch" in write));
    }
    /**
     * `plugin.wire.view(v).writeRows`: row operations on the property's last
     * direct write (or on its initial value), in as many patches as one
     * request carries, each applying on the one before. Needs a player that
     * takes row patches (`unsupported` otherwise).
     */
    directRows(property, ops) {
        const key = this.#directKey(property);
        if (!Array.isArray(ops))
            throw failure("invalid", "writeRows takes an array of row operations");
        return this.#queueDirect(key, { kind: "rows", ops: [...ops] }, false);
    }
    /** A direct write's property (Slint name), refused when `set()` or bound. */
    #directKey(property) {
        if (typeof property !== "string" || property === "")
            throw failure("invalid", "a property name is a non-empty string");
        const key = kebab(property);
        if (this.#values.has(key))
            throw failure("invalid", bothTracks(key, false));
        return key;
    }
    #queueDirect(key, job, whole) {
        if (!this.#target.live())
            return Promise.reject(failure("unavailable", `${key}: the view does not show (a window before show(), a panel or settings page before ready, or closed)`));
        let track = this.#direct.get(key);
        if (!track)
            this.#direct.set(key, (track = { base: undefined, queue: [], running: false }));
        const queued = track;
        return new Promise((resolve, reject) => {
            if (whole)
                for (const older of queued.queue.splice(0))
                    older.settle(false);
            queued.queue.push({
                ...job,
                settle: (ok, error) => error === undefined ? resolve(ok) : reject(error),
            });
            if (!queued.running)
                void this.#drainDirect(key, queued);
        });
    }
    /** Send one property's direct writes in order, one in flight. */
    async #drainDirect(key, track) {
        track.running = true;
        try {
            let job;
            while ((job = track.queue.shift())) {
                try {
                    if (!this.#target.live())
                        throw new Abandoned();
                    if (job.kind === "rows")
                        await this.#sendRows(key, track, job.ops);
                    else
                        await this.#sendDirect(key, track, job.write);
                    job.settle(true);
                }
                catch (error) {
                    job.settle(false, error instanceof Abandoned
                        ? failure("unavailable", `${key}: the view closed or showed its initial properties again during the write`)
                        : error);
                }
            }
        }
        finally {
            track.running = false;
        }
    }
    /**
     * One direct write, sent while the view shows what it showed when the
     * job started (`generation`): a failure after the view closed or showed
     * its initial properties again is not the write's ({@link Abandoned}, so
     * `unavailable`), and `true` says the player took it for the view that
     * is shown now, so its `seq` may serve as a base.
     */
    async #writeNow(key, write, generation) {
        if (!this.#target.live() || this.#generation !== generation)
            throw new Abandoned();
        try {
            await this.#target.write(key, write);
        }
        catch (error) {
            if (!this.#target.live() || this.#generation !== generation)
                throw new Abandoned();
            throw error;
        }
        return this.#target.live() && this.#generation === generation;
    }
    /**
     * A direct `PropertyWrite`: as given, with a `seq` and in parts where it
     * applies. A part the player answers after the view closed or showed its
     * initial properties again was not taken for the view shown now
     * ({@link Abandoned}, so `unavailable`, as for row operations).
     */
    async #sendDirect(key, track, write) {
        const generation = this.#generation;
        const send = async (part) => {
            if (!(await this.#writeNow(key, part, generation)))
                throw new Abandoned();
        };
        if ("patch" in write) {
            await send(write);
            const seq = write.patch?.seq;
            if (typeof seq === "number")
                track.base = seq;
            return;
        }
        if (!this.#patches) {
            await send(write);
            return;
        }
        const seq = typeof write.seq === "number" ? write.seq : ++this.#seq;
        const value = write.value;
        if (Array.isArray(value) &&
            write.more === undefined &&
            !JSON.stringify(value).includes(MARKDOWN_KEY)) {
            const sizes = value.map(rowBytes);
            const count = prefixWithin(sizes, CHUNK_BYTES);
            // In parts only when each row fits one request; otherwise whole, for
            // the player to take or refuse.
            if (count > 0 &&
                count < value.length &&
                sizes.every((size) => size <= CHUNK_BYTES)) {
                await send({ value: value.slice(0, count), seq, more: true });
                let last = seq;
                const insert = {
                    op: "insert",
                    at: count,
                    rows: value.slice(count),
                };
                const groups = chunkOps([insert]);
                for (const [i, ops] of groups.entries()) {
                    const next = ++this.#seq;
                    await send({
                        patch: { base: last, seq: next, ops },
                        ...(i < groups.length - 1 ? { more: true } : {}),
                    });
                    last = next;
                }
                track.base = last;
                return;
            }
        }
        // A base only for the view that is shown now: after a reopen the
        // player shows the property's default, not this write.
        await send({ ...write, seq });
        track.base = seq;
    }
    /**
     * Direct row operations: patches on the property's last direct write
     * since the view showed its initial properties. Without one the player
     * has no rows of the property's own to apply them on (a direct property
     * is never an initial property): refused with `stale` before anything is
     * sent, as the player would refuse it.
     */
    async #sendRows(key, track, ops) {
        if (!this.#patches)
            throw failure("unsupported", `${key}: row operations need a player that takes row patches (player 0.6.0)`);
        if (track.base === undefined)
            throw failure("stale", `${key}: no whole value was written directly since the view showed its initial properties; write the whole value first`);
        const generation = this.#generation;
        // Malformed operations go in one patch, for the player to refuse.
        const groups = wellFormed(ops) ? chunkOps(ops) : [[...ops]];
        for (const group of groups) {
            const seq = ++this.#seq;
            const current = await this.#writeNow(key, { patch: { base: track.base, seq, ops: group } }, generation);
            if (!current)
                throw new Abandoned();
            track.base = seq;
        }
    }
    /** Every property's value now, by Slint name. */
    values() {
        return Object.fromEntries(this.#values);
    }
    /** The host already shows these values (e.g. sent as initial properties). */
    shown(values) {
        this.#generation++;
        const patches = this.#patching;
        this.#sent = new Map(Object.entries(values).map(([k, v]) => [k, sentOf(v, patches)]));
        this.#bases = new Map(patches ? Object.keys(values).map((key) => [key, 0]) : []);
        this.#stales.clear();
        this.#forced.clear();
        for (const key of Object.keys(values))
            this.#failed.delete(key);
        // A property written directly shows its initial value again: its
        // next patch applies on 0.
        for (const track of this.#direct.values())
            track.base = undefined;
        // Not bound, a flush sends only what is marked: mark every value the
        // view does not show as it is now, such as a `set()` made while the view
        // was opening (after its initial properties were taken) or an initial
        // array cut to one request's size (`wired`), whose rest goes now:
        // only the rest, inserted on the initial rows (base 0), when the value
        // still begins with them (`#rest`).
        const cut = new Map(this.#cut);
        this.#cut.clear();
        this.#rest.clear();
        if (!this.#tracked)
            for (const [key, value] of this.#values) {
                if (!this.#encode(key, value).changed)
                    continue;
                this.#forced.add(key);
                const count = cut.get(key);
                const shown = values[key];
                if (count !== undefined &&
                    this.#patches &&
                    Array.isArray(value) &&
                    Array.isArray(shown) &&
                    shown.length === count &&
                    count < value.length)
                    // The rows shown, as encoded: the value is compared with them
                    // when it is sent (`#send`), not now; it may change before.
                    this.#rest.set(key, { count, enc: shown.map(encodeRow) });
            }
    }
    /** `value` encoded, and whether it differs from what was sent. */
    #encode(key, value) {
        const sent = this.#sent.get(key);
        if (isPrimitive(value)) {
            // Compared as it is: no `JSON.stringify` for the scalars of a pass.
            const changed = sent?.kind === "value"
                ? sent.value !== value
                : sent?.kind !== "json" || sent.enc !== JSON.stringify(value);
            return {
                key,
                value,
                changed,
                signature: () => JSON.stringify(value),
                sent: () => ({ kind: "value", value }),
            };
        }
        if (Array.isArray(value) && this.#patches) {
            // Row by row: the same work as one `JSON.stringify`, and the rows to
            // diff against those last sent. Rows that cannot change (deep-frozen
            // objects bound again, SDK 1.5.47) are taken from `settledRows`.
            const encoded = encodeRows(value);
            const { enc } = encoded;
            const signature = () => `[${enc.join(",")}]`;
            const changed = sent?.kind === "rows"
                ? !sameRows(sent.rows.enc, enc)
                : sent?.kind !== "json" || sent.enc !== signature();
            // Keys only for a write: an unchanged array costs its encoding alone.
            let keys;
            const keyed = () => (keys === undefined ? (keys = keyOf(value)) : keys);
            // Not bound: remembered as its JSON, for a later bind() to compare
            // with; no rows or keys are kept until patches can follow.
            const tracked = this.#tracked;
            return {
                key,
                value,
                changed,
                signature,
                rows: { ...encoded, keyed },
                sent: (counts = []) => tracked
                    ? {
                        kind: "rows",
                        rows: {
                            enc,
                            field: keyed()?.field ?? null,
                            keys: keyed()?.keys ?? null,
                            counts,
                        },
                    }
                    : { kind: "json", enc: signature() },
            };
        }
        const enc = JSON.stringify(value);
        return {
            key,
            value,
            changed: sent?.kind !== "json" || sent.enc !== enc,
            signature: () => enc,
            sent: () => ({ kind: "json", enc }),
        };
    }
    /**
     * Write one property: a row patch when the player takes them and one
     * applies (else the whole value); a `stale` patch is followed by the whole
     * value at once. The base is forgotten on any failure, so the next write
     * of the property is whole. A write larger than one request
     * ({@link CHUNK_BYTES} of rows, SDK 1.5.46) is sent in parts: a whole
     * array as its first rows and `insert` patches of the rest, a patch as
     * several, each applying on the one before.
     */
    async #send(pending) {
        const { key, value } = pending;
        if (!this.#patches) {
            await this.#target.write(key, { value: wireValue(value) });
            return pending.sent();
        }
        const generation = this.#generation;
        // An initial list cut to one request's size, not bound: the player shows
        // its first `rest` rows (the initial properties, base 0); only the
        // others go, as insert patches each applying on the one before.
        // Only while the value still begins with those rows, as encoded now
        // (a later `set()`, a deferred flush); a `stale` refusal of the first
        // patch falls back to the whole value below.
        const rest = this.#rest.get(key);
        this.#rest.delete(key);
        if (rest !== undefined &&
            Array.isArray(value) &&
            pending.rows &&
            !pending.rows.markdown &&
            rest.count < value.length &&
            sameRows(pending.rows.enc.slice(0, rest.count), rest.enc)) {
            const insert = {
                op: "insert",
                at: rest.count,
                rows: value
                    .slice(rest.count)
                    .map((row) => wireValue(row ?? null)),
            };
            tooLarge(key, [insert]);
            const [first, ...others] = chunkOps([insert]);
            const seq = ++this.#seq;
            let applied = false;
            try {
                await this.#target.write(key, {
                    patch: { base: 0, seq, ops: first },
                });
                applied = true;
            }
            catch (error) {
                if (!(error instanceof HostError && error.code === "stale"))
                    throw error;
            }
            if (applied) {
                await this.#parts(key, seq, others, false, generation);
                return pending.sent();
            }
        }
        const base = this.#bases.get(key);
        this.#bases.delete(key);
        const sent = this.#sent.get(key);
        let plan = base !== undefined &&
            sent?.kind === "rows" &&
            pending.rows &&
            (this.#stales.get(key) ?? 0) < 2
            ? rowPatch(sent.rows, pending.rows, value)
            : null;
        // A patch sent in several never passes the row limit midway (the diff
        // removes before it inserts; checked anyway); otherwise whole.
        if (plan && sent?.kind === "rows") {
            let length = sent.rows.enc.length;
            for (const op of plan.ops) {
                if (op.op === "insert")
                    length += op.rows.length;
                else if (op.op === "remove")
                    length -= op.count;
                if (length > UI_VALUE_LIMITS.arrayRows) {
                    plan = null;
                    break;
                }
            }
        }
        if (plan) {
            tooLarge(key, plan.ops);
            const [first, ...rest] = chunkOps(plan.ops);
            const seq = ++this.#seq;
            let applied = false;
            try {
                await this.#target.write(key, {
                    patch: { base: base, seq, ops: first },
                });
                applied = true;
            }
            catch (error) {
                if (!(error instanceof HostError && error.code === "stale"))
                    throw error;
                this.#stales.set(key, (this.#stales.get(key) ?? 0) + 1);
            }
            if (applied) {
                this.#stales.delete(key);
                await this.#parts(key, seq, rest, false, generation);
                return pending.sent(plan.counts);
            }
        }
        const seq = ++this.#seq;
        // Rows holding Markdown documents go whole, as before: the player
        // merges documents in place and takes no parts of them.
        if (Array.isArray(value) && pending.rows && !pending.rows.markdown) {
            const sizes = pending.rows.enc.map(jsonBytes);
            const count = prefixWithin(sizes, CHUNK_BYTES);
            if (count < value.length) {
                const rows = value;
                // Over the limits, or a row larger than one request: refused here,
                // as the player would refuse the whole value, before any part is
                // sent (a part refused midway would leave the list half sent).
                const problem = valueLimitProblem(wireValue(value));
                if (problem)
                    throw new HostError("limit", `${key}: ${problem}`);
                const largest = sizes.reduce((a, b) => Math.max(a, b), 0);
                if (largest > CHUNK_BYTES)
                    throw rowTooLarge(key, largest);
                // Every part but the last carries `more`: the player holds them
                // and writes the whole value at the last, so a list it shows is
                // written once (player 0.6.0).
                await this.#target.write(key, {
                    value: wireValue(rows.slice(0, count)),
                    seq,
                    more: true,
                });
                const insert = {
                    op: "insert",
                    at: count,
                    rows: rows.slice(count).map((row) => wireValue(row ?? null)),
                };
                await this.#parts(key, seq, chunkOps([insert]), true, generation);
                return pending.sent();
            }
        }
        await this.#target.write(key, { value: wireValue(value), seq });
        // A base for the next patch, once the view is bound.
        if (this.#tracked)
            this.#bases.set(key, seq);
        return pending.sent();
    }
    /**
     * The rest of a write sent in parts: each patch applies on the one
     * before (`base`), and the property's base is the last one's; with
     * `more`, every part but the last says so. A failure leaves no base: the
     * next write of the property is whole. When the view closed or showed its
     * initial properties again meanwhile (`generation`), the rest is dropped
     * quietly ({@link Abandoned}): the view's next flush sends what it shows.
     */
    async #parts(key, base, groups, more, generation) {
        let last = base;
        for (const [i, ops] of groups.entries()) {
            if (!this.#target.live() || this.#generation !== generation)
                throw new Abandoned();
            const seq = ++this.#seq;
            try {
                await this.#target.write(key, {
                    patch: { base: last, seq, ops },
                    ...(more && i < groups.length - 1 ? { more: true } : {}),
                });
            }
            catch (error) {
                // Refused because the view went meanwhile: not a failure.
                if (!this.#target.live() || this.#generation !== generation)
                    throw new Abandoned();
                throw error;
            }
            last = seq;
        }
        if (this.#tracked)
            this.#bases.set(key, last);
    }
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
    flush() {
        if (this.#flushing || this.#retry || !this.#target.live())
            return;
        this.#flushing = true;
        void (async () => {
            let failed = false;
            let failure;
            try {
                let pending;
                do {
                    pending = [];
                    // Not bound: only what `set()` named is sent (whole), nothing is
                    // compared. Bound: every value against what was sent.
                    const keys = this.#tracked ? this.#values.keys() : [...this.#forced];
                    for (const key of keys) {
                        if (!this.#values.has(key))
                            continue;
                        const value = this.#values.get(key);
                        const encoded = this.#encode(key, value);
                        const failedAs = this.#failed.get(key);
                        if (failedAs !== undefined && failedAs === encoded.signature())
                            continue;
                        if (this.#forced.has(key) || encoded.changed)
                            pending.push(encoded);
                    }
                    for (const encoded of pending) {
                        if (!this.#target.live())
                            return;
                        const key = encoded.key;
                        this.#forced.delete(key);
                        try {
                            this.#sent.set(key, await this.#send(encoded));
                            this.#failed.delete(key);
                            this.#backoff = 0;
                            this.#broken = false;
                        }
                        catch (error) {
                            // The view closed or showed its initial properties again while
                            // a write was sent in parts: nothing failed (the next flush
                            // sends what the view shows).
                            if (error instanceof Abandoned) {
                                queueMicrotask(() => this.flush());
                                return;
                            }
                            failed = true;
                            // Report the start of a failing streak only.
                            if (!this.#broken)
                                failure ??= error;
                            this.#broken = true;
                            this.#failed.set(key, encoded.signature());
                        }
                    }
                } while (pending.length && !failed && this.#target.live());
            }
            finally {
                this.#flushing = false;
            }
            if (!failed)
                return;
            this.#backoff = Math.min(Math.max(250, this.#backoff * 2), 30_000);
            this.#retry = setTimeout(() => {
                this.#retry = undefined;
                this.flush();
            }, this.#backoff);
            this.#retry.unref?.();
            if (failure !== undefined)
                this.#target.report(failure);
        })();
    }
    /** The view closed: no retry after a back-off, no direct write waiting. */
    stop() {
        clearTimeout(this.#retry);
        for (const track of this.#direct.values())
            for (const job of track.queue.splice(0))
                job.settle(false);
    }
}
