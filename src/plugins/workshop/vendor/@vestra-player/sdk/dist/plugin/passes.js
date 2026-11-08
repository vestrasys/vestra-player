/**
 * The state each event keeps current, and whether `plugin.when` can say
 * the same without the plugin running.
 */
const STATE = {
    "source.state": { name: "plugin.player.state", when: true },
    "window.state": { name: "plugin.window.state", when: true },
    "appearance.changed": { name: "plugin.appearance", when: true },
    "timeline.changed": { name: "plugin.timeline.marks", when: false },
    "commands.keys": { name: "plugin.commands.keys", when: false },
    "playback.backend": { name: "plugin.subtitles.state", when: false },
    "resolvers.fileTypes": {
        name: "plugin.resolvers.fileTypes",
        when: false,
    },
};
/** Whether something that read `deps` may see a different result after `change`. */
function affected(deps, change) {
    for (const [key, fields] of change) {
        const read = deps.get(key);
        if (!read)
            continue;
        if (read.size === 0)
            return true;
        for (const field of fields)
            if (read.has(field))
                return true;
    }
    return false;
}
/** Add `from` to `into` (an empty set, the whole state, wins). */
function merge(into, from) {
    for (const [key, fields] of from) {
        const set = into.get(key);
        if (!set)
            into.set(key, new Set(fields));
        else if (set.size && fields.size)
            for (const field of fields)
                set.add(field);
        else
            set.clear();
    }
}
/**
 * The longest a binding waits for a full pass while precise passes skip
 * it: state a plugin changes outside code the SDK runs (its own timer,
 * without `update()`) still shows within about this long.
 */
export const SAFETY_PASS_MS = 1000;
/** The update pass scheduler of one plugin (`Runtime` delegates to it). */
export class Passes {
    /** The plugin's connection: holds the events a binding reads. */
    #connection;
    /** Each binding with the events whose state it read last time (held). */
    #bindings = new Map();
    /** The events read by the binding being evaluated, with who read them. */
    #reads;
    /** Who reads now (a command's field and where it was added). */
    #reader;
    #scheduled = false;
    /** The next pass evaluates every binding (not only a state copy changed). */
    #full = false;
    /** State copy changes since the last pass (a precise pass, unless `#full`). */
    #pending = new Map();
    /** The change the running precise pass is for; `undefined` in a full pass. */
    #pass;
    /** The state copy fields read by what is being evaluated now. */
    #deps;
    /** Each part's reads and fields last time (`part`). */
    #parts = new WeakMap();
    /** A precise pass skipped something: a full pass follows (`SAFETY_PASS_MS`). */
    #safety;
    /** The running pass skipped a binding or a part. */
    #skipped = false;
    /** The plugin was disposed: no safety pass is armed any more. */
    #stopped = false;
    /** Pass counters, read by the SDK's own tests and benchmark ({@link PassCounts}). */
    counts = {
        passes: 0,
        fullPasses: 0,
        evaluations: 0,
        parts: 0,
        skippedParts: 0,
        safetyPasses: 0,
    };
    constructor(connection) {
        this.#connection = connection;
    }
    bind(evaluate) {
        const held = new Map();
        const binding = { held, deps: new Map() };
        this.#bindings.set(evaluate, binding);
        this.invalidate();
        return {
            dispose: async () => {
                if (this.#bindings.get(evaluate) !== binding)
                    return;
                this.#bindings.delete(evaluate);
                for (const hold of held.values())
                    await hold.dispose();
                held.clear();
            },
        };
    }
    /**
     * A service's state for `event` is read: a binding evaluated now depends
     * on it, so the event counts as listened to while the binding reads it.
     */
    reading(event) {
        const reads = this.#reads;
        if (!reads || reads.has(event))
            return;
        const reader = this.#reader;
        const state = STATE[event];
        reads.set(event, () => {
            const read = state?.name ?? event;
            const site = reader?.where?.();
            const at = site ? ` (${site})` : "";
            if (!reader)
                return `a binding reads ${read}`;
            if (!reader.field)
                return `${reader.label} reads ${read}${at}`;
            const hint = state?.when ? " — use plugin.when" : "";
            return `${reader.label} is a function that reads ${read}${at}${hint}`;
        });
    }
    /**
     * A field of the SDK's copy of a state (`key`) is read; without `field`,
     * the state is taken (`plugin.player.state`). What is being evaluated
     * then depends on it: a precise pass after a change of that field runs
     * it again.
     */
    depends(key, field) {
        const deps = this.#deps;
        if (!deps)
            return;
        let fields = deps.get(key);
        if (!fields)
            deps.set(key, (fields = new Set()));
        if (field !== undefined)
            fields.add(field);
    }
    /**
     * Evaluate one part of the binding being evaluated (one `view.bind`
     * function), identified by `token`. A precise pass skips it when nothing
     * it read last time changed, and returns `undefined`; what it read then
     * still counts for the binding.
     */
    part(token, read) {
        const last = this.#parts.get(token);
        const outerReads = this.#reads;
        const outerDeps = this.#deps;
        if (last && this.#pass && !affected(last.deps, this.#pass)) {
            this.#skipped = true;
            this.counts.skippedParts++;
            if (outerReads)
                for (const [event, why] of last.reads)
                    if (!outerReads.has(event))
                        outerReads.set(event, why);
            if (outerDeps)
                merge(outerDeps, last.deps);
            return undefined;
        }
        this.counts.parts++;
        const reads = new Map();
        const deps = new Map();
        this.#reads = outerReads && reads;
        this.#deps = deps;
        try {
            return { value: read() };
        }
        finally {
            this.#reads = outerReads;
            this.#deps = outerDeps;
            this.#parts.set(token, { reads, deps });
            if (outerReads)
                for (const [event, why] of reads)
                    if (!outerReads.has(event))
                        outerReads.set(event, why);
            if (outerDeps)
                merge(outerDeps, deps);
        }
    }
    /**
     * Run plugin code (a state listener) and return the state copy fields it
     * read; they are not counted as a binding's.
     */
    observe(run) {
        const outerReads = this.#reads;
        const outerDeps = this.#deps;
        const deps = new Map();
        this.#reads = undefined;
        this.#deps = deps;
        try {
            return { value: run(), deps };
        }
        finally {
            this.#reads = outerReads;
            this.#deps = outerDeps;
        }
    }
    /**
     * Whether code that read `deps` while `change` was delivered needs a full
     * pass after it: it read a field that changed, or no field of that state
     * at all (so the SDK cannot tell what it depends on).
     */
    needsFull(deps, change) {
        for (const key of change.keys()) {
            const read = deps.get(key);
            if (!read || read.size === 0)
                return true;
        }
        return affected(deps, change);
    }
    /**
     * Evaluate `read` as `label`, added at `where`: the state it reads is
     * reported as read by it. `field`: a command's `enabled` or `checked`,
     * which a `plugin.when` condition can replace.
     */
    reader(label, where, read, field = false) {
        const outer = this.#reader;
        this.#reader = { label, where, field };
        try {
            return read();
        }
        finally {
            this.#reader = outer;
        }
    }
    /** Evaluate one binding, holding the events whose state it read. */
    #evaluate(evaluate, binding) {
        const held = binding.held;
        const reads = new Map();
        const deps = new Map();
        const outer = this.#reads;
        const outerDeps = this.#deps;
        const reader = this.#reader;
        this.#reads = reads;
        this.#deps = deps;
        this.#reader = undefined;
        this.counts.evaluations++;
        try {
            evaluate();
        }
        finally {
            this.#reads = outer;
            this.#deps = outerDeps;
            this.#reader = reader;
            binding.deps = deps;
            for (const [event, hold] of held)
                if (!reads.has(event)) {
                    held.delete(event);
                    void hold.dispose();
                }
            for (const [event, why] of reads)
                if (!held.has(event))
                    held.set(event, this.#connection.hold(event, why));
        }
    }
    /**
     * A full pass: every binding is evaluated. Requested after plugin code
     * the SDK ran, by `plugin.update()`, and on any change other than a state
     * copy's fields. Coalesced: one pass per microtask turn, however many
     * requests.
     */
    invalidate() {
        this.#full = true;
        this.#schedule();
    }
    /**
     * Fields of the SDK's copy of a state (`key`) changed: a precise pass
     * evaluates only the bindings (and view bind functions) that read one of
     * them, unless a full pass is due anyway.
     */
    changed(key, fields) {
        let set = this.#pending.get(key);
        if (!set)
            this.#pending.set(key, (set = new Set()));
        for (const field of fields)
            set.add(field);
        this.#schedule();
    }
    #schedule() {
        if (this.#scheduled)
            return;
        this.#scheduled = true;
        queueMicrotask(() => this.#run());
    }
    /**
     * One pass. A precise pass that skips anything arms a full pass
     * `SAFETY_PASS_MS` later (unless a full pass comes first), for plugin
     * state changed outside code the SDK ran.
     */
    #run() {
        this.#scheduled = false;
        const full = this.#full;
        const change = this.#pending;
        this.#full = false;
        this.#pending = new Map();
        this.counts.passes++;
        if (full)
            this.counts.fullPasses++;
        if (full) {
            clearTimeout(this.#safety);
            this.#safety = undefined;
        }
        this.#skipped = false;
        for (const [evaluate, binding] of [...this.#bindings]) {
            if (this.#bindings.get(evaluate) !== binding)
                continue;
            if (!full && !affected(binding.deps, change)) {
                this.#skipped = true;
                continue;
            }
            this.#pass = full ? undefined : change;
            try {
                this.#evaluate(evaluate, binding);
            }
            catch (error) {
                console.error(`[${this.#connection.manifest.id}] binding failed:`, error);
            }
            finally {
                this.#pass = undefined;
            }
        }
        if (this.#skipped && !this.#safety && !this.#stopped) {
            this.#safety = setTimeout(() => {
                this.#safety = undefined;
                this.counts.safetyPasses++;
                this.invalidate();
            }, SAFETY_PASS_MS);
            this.#safety.unref?.();
        }
    }
    /** The plugin was disposed: cancel the pending safety pass, arm no more. */
    stop() {
        this.#stopped = true;
        clearTimeout(this.#safety);
        this.#safety = undefined;
    }
}
