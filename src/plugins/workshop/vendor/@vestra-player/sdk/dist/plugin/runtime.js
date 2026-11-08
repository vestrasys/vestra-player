/**
 * Shared machinery behind every scope: the connection to the player
 * (`connection.ts`, L1) with the update passes (`passes.ts`, L2) on top.
 * `Runtime` is the connection plus the pass methods, delegated, so every
 * caller keeps `runtime.host`, `runtime.on`, `runtime.bind` and the rest.
 */
import { Connection } from "./connection.js";
import { Passes } from "./passes.js";
// Internal, kept where it was before 1.6.19: the player's host-agreement test
// reads WAKE_EVENTS from dist/plugin/runtime.js.
export { WAKE_EVENTS } from "./connection.js";
/** Per-plugin services: the connection, and the update passes over it. */
export class Runtime extends Connection {
    /** The update pass scheduler. */
    passes;
    /** Pass counters, read by the SDK's own tests and benchmark ({@link PassCounts}). */
    counts;
    constructor(transport, manifest, directory, dataDirectory, app, options = {}) {
        super(transport, manifest, directory, dataDirectory, app, options);
        this.passes = new Passes(this);
        this.counts = this.passes.counts;
    }
    /** {@link Passes.bind} */
    bind(evaluate) {
        return this.passes.bind(evaluate);
    }
    /** {@link Passes.reading} */
    reading(event) {
        this.passes.reading(event);
    }
    /** {@link Passes.depends} */
    depends(key, field) {
        this.passes.depends(key, field);
    }
    /** {@link Passes.part} */
    part(token, read) {
        return this.passes.part(token, read);
    }
    /** {@link Passes.observe} */
    observe(run) {
        return this.passes.observe(run);
    }
    /** {@link Passes.needsFull} */
    needsFull(deps, change) {
        return this.passes.needsFull(deps, change);
    }
    /** {@link Passes.reader} */
    reader(label, where, read, field = false) {
        return this.passes.reader(label, where, read, field);
    }
    /** {@link Passes.invalidate} */
    invalidate() {
        this.passes.invalidate();
    }
    /** {@link Passes.changed} */
    changed(key, fields) {
        this.passes.changed(key, fields);
    }
    /** {@link Passes.stop} */
    stop() {
        this.passes.stop();
    }
}
