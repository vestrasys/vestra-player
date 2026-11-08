/**
 * `ViewBase`: what windows, panels and settings pages share — callbacks, and
 * a {@link ViewWriter} (L1) with a {@link ViewBinder} (L2) over it. Every
 * view joins the update passes when made (doc/specs/sdk-refactor.md §1.2 in
 * the player repository; Step 2 moves that to the first `bind()`).
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { PATCH_FEATURE } from "../patch.js";
import { ViewBinder } from "./binder.js";
import { kebab, ViewWriter } from "./writer.js";
/** Property and callback plumbing shared by windows, panels and settings pages. */
export class ViewBase {
    scope;
    handlers = new Map();
    #writer;
    #binder;
    #binding;
    closed = false;
    constructor(scope) {
        this.scope = scope;
        this.#writer = new ViewWriter({
            live: () => this.live(),
            write: (property, write) => this.write(property, write),
            report: (error) => this.scope.report(error),
            patches: () => this.scope.runtime.features.has(PATCH_FEATURE),
        });
        this.#binder = new ViewBinder(scope, this.#writer, () => this.closed);
        this.#binding = scope.runtime.bind(() => this.evaluate());
    }
    /** {@link ViewWriter.wired}: initial properties in wire form. */
    wired(values, refuse = false) {
        return this.#writer.wired(values, refuse);
    }
    on(callback, handler) {
        const name = kebab(callback);
        this.handlers.set(name, handler);
        return this.scope.onDispose(() => {
            if (this.handlers.get(name) === handler)
                this.handlers.delete(name);
        });
    }
    set(props) {
        this.#writer.set(props);
    }
    bind(props) {
        return this.#binder.bind(props);
    }
    /** {@link ViewBinder.snapshot}: current desired properties. */
    snapshot() {
        return this.#binder.snapshot();
    }
    evaluate() {
        this.#binder.evaluate();
    }
    /** The host already shows these values (e.g. sent as initial properties). */
    shown(values) {
        this.#writer.shown(values);
    }
    /** {@link ViewWriter.flush}: send what changed. */
    flush() {
        this.#writer.flush();
    }
    dispatch(callback, args) {
        if (this.closed)
            return;
        const handler = this.handlers.get(callback);
        if (handler)
            this.scope.run(handler, ...args);
        else
            this.scope.update();
    }
    async release() {
        this.closed = true;
        this.#writer.stop();
        await this.#binding.dispose();
    }
}
/** Find `export component X` / `export global Y` so most mounts need no names. */
export function exported(directory, file, kind) {
    const text = readFileSync(resolve(directory, file), "utf8");
    const names = [
        ...text.matchAll(new RegExp(`export\\s+${kind}\\s+([\\w-]+)`, "g")),
    ].map((m) => m[1]);
    if (names.length !== 1)
        throw new Error(`${file}: name the ${kind} to mount (found ${names.length})`);
    return names[0];
}
