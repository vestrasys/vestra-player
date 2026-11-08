/**
 * The update passes (L2, refactor plan §2.2): bindings, the state copy
 * fields they read (`reading`, `depends`, `part`, `observe`, `reader`),
 * full and precise passes (`invalidate`, `changed`) and the safety pass.
 * It uses the connection (`hold`) and nothing it builds on imports it
 * (`scripts/check-layers.mjs`).
 */
import type { Connection } from "./connection.ts";
import type { Disposable } from "./types.ts";
/**
 * The fields of the SDK's state copies that a binding or listener read, by
 * state (`player`: `plugin.player.state`). An empty set: the state was
 * taken whole (no field read was seen), so any change of it counts.
 */
export type Deps = Map<string, Set<string>>;
/** A change of the SDK's state copies: the fields that changed, by state. */
export type Change = ReadonlyMap<string, ReadonlySet<string>>;
/**
 * The longest a binding waits for a full pass while precise passes skip
 * it: state a plugin changes outside code the SDK runs (its own timer,
 * without `update()`) still shows within about this long.
 */
export declare const SAFETY_PASS_MS = 1000;
/**
 * The most passes in a row that a pass itself asks for (something it
 * evaluated changed what it reads, `plugin.state` written in a `bind`
 * function). The next one is dropped and reported once: without it such a
 * plugin would spin in microtasks and never yield. It counts passes, not
 * loops: a legitimate chain deeper than this (101 bindings, each writing a
 * `plugin.state` field the next one reads) is cut the same way: the
 * readers of the dropped pass's changes run again only in the next full
 * pass (after a handler, `update()`).
 */
export declare const MAX_CHAINED_PASSES = 100;
/**
 * Read-only counters of the update passes, for the SDK's own
 * characterisation tests and benchmark (`test/golden-traces.test.ts`,
 * `scripts/bench.mjs`): how many passes ran, how many were full, how many
 * bindings and view bind functions (parts) were evaluated or skipped. Not
 * part of the plugin API; counting changes no behaviour.
 */
export interface PassCounts {
    /** Passes run (`Passes.#run`), precise and full. */
    passes: number;
    /** Of them, full passes (every binding evaluated). */
    fullPasses: number;
    /** Bindings evaluated (`Passes.#evaluate`). */
    evaluations: number;
    /** View bind functions evaluated inside a binding (`Passes.part`). */
    parts: number;
    /** View bind functions a precise pass skipped (`Passes.part`). */
    skippedParts: number;
    /**
     * Full passes the safety timer requested (`SAFETY_PASS_MS` after a
     * precise pass skipped something), counted when the timer fires.
     */
    safetyPasses: number;
}
/**
 * One `Passes.bind` registration as its owner holds it: dispose it, or ask
 * for it alone to be evaluated again ({@link BindingHandle.invalidate}).
 */
export interface BindingHandle extends Disposable {
    /**
     * Evaluate this binding in the next pass even if nothing it read
     * changed (a precise pass, not a full one): its owner changed something
     * the binding reads that no state copy tracks (`Command.set`).
     */
    invalidate(): void;
}
/** The update pass scheduler of one plugin (`Runtime` delegates to it). */
export declare class Passes {
    #private;
    /** Where a runaway goes: the plugin's error handlers (`createPlugin`). */
    report: (error: unknown) => void;
    /** Pass counters, read by the SDK's own tests and benchmark ({@link PassCounts}). */
    readonly counts: PassCounts;
    constructor(connection: Connection);
    /**
     * Register a binding: `evaluate` runs in a full pass now and in every
     * pass that concerns it. Only what calls this takes part in the passes:
     * a view on its first `bind()`, a command with a function field, a
     * sequence provider (SDK 1.7.0; before, every view and command).
     */
    bind(evaluate: () => void, slot?: number): BindingHandle;
    /**
     * A place in the evaluation order, for a binding made later (`bind`): a
     * view or a command reserves one when it is made, so it is evaluated in
     * the order of making whenever it joins.
     */
    slot(): number;
    /**
     * A service's state for `event` is read: a binding evaluated now depends
     * on it, so the event counts as listened to while the binding reads it.
     */
    reading(event: string): void;
    /**
     * A field of the SDK's copy of a state (`key`) is read; without `field`,
     * the state is taken (`plugin.player.state`). What is being evaluated
     * then depends on it: a precise pass after a change of that field runs
     * it again.
     */
    depends(key: string, field?: string): void;
    /**
     * Evaluate one part of the binding being evaluated (one `view.bind`
     * function), identified by `token`. A precise pass skips it when nothing
     * it read last time changed, and returns `undefined`; what it read then
     * still counts for the binding.
     */
    part<T>(token: object, read: () => T): {
        value: T;
    } | undefined;
    /**
     * Run plugin code (a state listener) and return the state copy fields it
     * read; they are not counted as a binding's.
     */
    observe<T>(run: () => T): {
        value: T;
        deps: Deps;
    };
    /**
     * Whether code that read `deps` while `change` was delivered needs a full
     * pass after it: it read a field that changed, or no field of that state
     * at all (so the SDK cannot tell what it depends on).
     */
    needsFull(deps: Deps, change: Change): boolean;
    /**
     * Evaluate `read` as `label`, added at `where`: the state it reads is
     * reported as read by it. `field`: a command's `enabled` or `checked`,
     * which a `plugin.when` condition can replace.
     */
    reader<T>(label: string, where: (() => string | undefined) | undefined, read: () => T, field?: boolean): T;
    /**
     * A full pass: every binding is evaluated. Requested after plugin code
     * the SDK ran, by `plugin.update()`, and on any change other than a state
     * copy's fields. Coalesced: one pass per microtask turn, however many
     * requests.
     */
    invalidate(): void;
    /**
     * Fields of the SDK's copy of a state (`key`) changed: a precise pass
     * evaluates only the bindings (and view bind functions) that read one of
     * them, unless a full pass is due anyway.
     */
    changed(key: string, fields: Iterable<string>): void;
    /** The plugin was disposed: cancel the pending safety pass, arm no more. */
    stop(): void;
}
