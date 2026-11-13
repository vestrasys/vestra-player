/**
 * `plugin.mpv`: the mpv pass-through (L1). The checks here are the
 * player's own, made early with the same table (`src/mpv.ts`); what only
 * the player can tell (another plugin's label, mpv's answer) is its.
 */
import { mpvCommandProblem, mpvCommandRefusal, mpvNameProblem, mpvPropertyRefusal, } from "../mpv.js";
import { failure } from "./errors.js";
/** One per plugin: the properties observed and their listeners. */
class Observers {
    #runtime;
    #names = new Map();
    #started = false;
    constructor(runtime) {
        this.#runtime = runtime;
    }
    /** One `mpv.observe` per name, however many listeners. */
    add(name, listener, report) {
        if (!this.#started) {
            this.#started = true;
            // Sent to the plugin that observes: no subscription of its own.
            this.#runtime.raw("mpv.property", (data) => {
                const event = data;
                for (const each of [...(this.#names.get(event.name) ?? [])])
                    each(event.value);
            });
        }
        let listeners = this.#names.get(name);
        if (!listeners) {
            listeners = new Set();
            this.#names.set(name, listeners);
            this.#runtime.starting(this.#runtime.host.mpv.observeProperty(name), report);
        }
        listeners.add(listener);
    }
    async remove(name, listener) {
        const listeners = this.#names.get(name);
        if (!listeners?.delete(listener) || listeners.size)
            return;
        this.#names.delete(name);
        await this.#runtime.host.mpv.unobserveProperty(name).catch(() => { });
    }
}
export const mpv = (scope) => {
    /** Refuse at once without `mpv`, as the player does. */
    const declared = () => {
        if (!scope.runtime.can("mpv"))
            throw failure("capability", "mpv must be declared in manifest");
    };
    const named = (name, write) => {
        declared();
        const problem = mpvNameProblem(name);
        if (problem)
            throw failure("invalid", problem);
        const refused = mpvPropertyRefusal(name, write);
        if (refused)
            throw failure("refused", refused);
        return name;
    };
    const observers = () => scope.runtime.service("mpv.observers", () => new Observers(scope.runtime));
    return {
        async command(args) {
            declared();
            const problem = mpvCommandProblem(args);
            if (problem)
                throw failure("invalid", problem);
            const refused = mpvCommandRefusal(args);
            if (refused)
                throw failure("refused", refused);
            const answer = await scope.runtime.host.mpv.command([...args]);
            return (answer?.result ?? null);
        },
        async get(name) {
            const answer = await scope.runtime.host.mpv.get(named(name, false));
            return (answer?.value ?? null);
        },
        async set(name, value) {
            const property = named(name, true);
            if (value === undefined)
                throw failure("invalid", "set takes a value");
            await scope.runtime.host.mpv.set(property, value);
        },
        observe(name, listener) {
            const property = named(name, false);
            if (typeof listener !== "function")
                throw failure("invalid", "observe takes a property name and a listener");
            // Like `player.on`: no full update pass after it.
            const deliver = (value) => scope.runQuiet(listener, value);
            observers().add(property, deliver, (error) => scope.report(error));
            const handle = {
                dispose: async () => {
                    scope.forget(handle);
                    await observers().remove(property, deliver);
                },
            };
            return scope.add(handle);
        },
    };
};
