/** Shared machinery behind every scope: invocation, update passes, events. */
import { createWire, HostError } from "../wire.js";
import { LOCALES } from "../i18n.js";
import { basename, isAbsolute, relative } from "node:path";
import { fileURLToPath } from "node:url";
/** The SDK's own files (`src/` or `dist/`): left out of a call site. */
const SDK_DIRECTORY = fileURLToPath(new URL("..", import.meta.url));
/**
 * Events the player wakes a stopped plugin for (player 0.5.21): a listener
 * of one does not keep the plugin running. They follow deliberate changes
 * (an install, a key edit, the palette, a resolver), not playback.
 */
export const WAKE_EVENTS = [
    "packages.changed",
    "bindings.changed",
    "appearance.changed",
    "resolvers.fileTypes",
];
/**
 * The state each event keeps current, and whether `plugin.when` can say
 * the same without the plugin running.
 */
const STATE = {
    "source.state": { name: "plugin.player.state", when: true },
    "window.state": { name: "plugin.window.state", when: true },
    "appearance.changed": { name: "plugin.appearance", when: true },
    "timeline.changed": { name: "plugin.timeline.marks", when: false },
    "playback.backend": { name: "plugin.subtitles.state", when: false },
    "resolvers.fileTypes": {
        name: "plugin.resolvers.fileTypes",
        when: false,
    },
};
export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
export function toError(error) {
    if (error instanceof Error) {
        const coded = error;
        if (typeof coded.code !== "string")
            Object.defineProperty(coded, "code", {
                value: error.name === "AbortError" ? "aborted" : "error",
                enumerable: false,
                writable: true,
            });
        return coded;
    }
    return new HostError("error", String(error));
}
export const failure = (code, message = code) => new HostError(code, message);
/** Host replies are definite; anything else (lost reply, client timeout) is not. */
export const uncertain = (error) => !(error instanceof HostError) || error.code === "timeout";
/** Mark a promise as observed so an unused `ready` never crashes the runner. */
export function quiet(promise) {
    promise.catch(() => { });
    return promise;
}
export const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
/** One value in flight, only the newest waiting. Each push learns its fate. */
export class Latest {
    #queued = [];
    #running = false;
    #stopped = false;
    #send;
    constructor(send) {
        this.#send = send;
    }
    push(value) {
        if (this.#stopped)
            return Promise.resolve(false);
        return new Promise((resolve, reject) => {
            for (const older of this.#queued.splice(0))
                older.settle(false);
            this.#queued.push({
                value,
                settle: (ok, error) => error === undefined ? resolve(ok) : reject(error),
            });
            if (!this.#running)
                void this.#drain();
        });
    }
    stop() {
        this.#stopped = true;
        for (const older of this.#queued.splice(0))
            older.settle(false);
    }
    async #drain() {
        this.#running = true;
        try {
            let next;
            while ((next = this.#queued.shift()) && !this.#stopped) {
                try {
                    await this.#send(next.value);
                    next.settle(true);
                }
                catch (error) {
                    next.settle(false, error);
                }
            }
        }
        finally {
            this.#running = false;
        }
    }
}
/**
 * Per-plugin services. Scopes are thin views over this object; the flat RPC
 * clients stay private here so protocol fields never reach plugin code.
 */
export class Runtime {
    host;
    manifest;
    directory;
    /** Writable per-plugin data directory from the host, if it provided one. */
    dataDirectory;
    /** Test doubles for local processes; the real system when absent. */
    local;
    /** Registrations that activation must wait for before reporting ready. */
    startup = new Set();
    #activating = true;
    #transport;
    /** Each binding with the events whose state it read last time (held). */
    #bindings = new Map();
    /** The events read by the binding being evaluated, with who read them. */
    #reads;
    /** Who reads now (a command's field and where it was added). */
    #reader;
    #scheduled = false;
    #events = new Map();
    views = new Map();
    commands = new Map();
    services = new Map();
    /** `plugin.app`, from the start-up handshake; `system()` asks the player. */
    app;
    /** `plugin.i18n.locale`, from the start-up handshake; English when unknown. */
    locale;
    /**
     * Protocol additions the player advertises at start-up (`features` in the
     * handshake's app data, player 0.6.0): `ui.patch` (row patches of
     * property writes). Not part of the plugin API; empty for older players.
     */
    features;
    constructor(transport, manifest, directory, dataDirectory, app) {
        this.#transport = transport;
        this.dataDirectory = dataDirectory;
        this.local = transport.local;
        this.host = createWire(transport);
        const host = this.host;
        this.app = appInfo(app, () => host.app.system());
        const features = app
            ?.features;
        this.features = new Set(Array.isArray(features)
            ? features.filter((name) => typeof name === "string")
            : []);
        this.locale = LOCALES.includes(transport.locale ?? "")
            ? transport.locale
            : "en";
        this.manifest = manifest;
        this.directory = directory;
        transport.on("ui.callback", (data) => {
            const event = data;
            this.views.get(event.view)?.(event.callback, event.args);
        });
        transport.on("commands.execute", (data) => {
            this.commands.get(data.id)?.();
        });
    }
    can(capability) {
        return this.manifest.capabilities.includes(capability);
    }
    /** Lazily created singletons shared by every scope of this plugin. */
    service(name, create) {
        if (!this.services.has(name))
            this.services.set(name, create());
        return this.services.get(name);
    }
    /**
     * Registrations started while `activate` runs must succeed before the
     * plugin reports ready; later ones report failures to their scope.
     */
    starting(work, report) {
        if (this.#activating)
            this.startup.add(work);
        else
            work.catch(report);
        work.catch(() => { });
        return work;
    }
    async activated() {
        while (this.startup.size) {
            const pending = [...this.startup];
            this.startup.clear();
            for (const result of await Promise.allSettled(pending))
                if (result.status === "rejected")
                    throw result.reason;
        }
        this.#activating = false;
    }
    bind(evaluate) {
        const held = new Map();
        this.#bindings.set(evaluate, held);
        this.invalidate();
        return {
            dispose: async () => {
                if (this.#bindings.get(evaluate) !== held)
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
    /**
     * Where the plugin's code called into the SDK: the first stack frame
     * outside the SDK, as a path relative to the plugin's directory and a
     * line (`main.ts:353`). Captured now, formatted when asked.
     */
    callSite() {
        const limit = Error.stackTraceLimit;
        Error.stackTraceLimit = 40;
        const error = new Error();
        Error.stackTraceLimit = limit;
        let site = null;
        return () => {
            if (site === null)
                site = this.#site(error.stack ?? "");
            return site;
        };
    }
    #site(stack) {
        for (const line of stack.split("\n").slice(1)) {
            const match = /\(?((?:file:\/\/)?[^()\s]+?):(\d+):\d+\)?\s*$/.exec(line);
            if (!match)
                continue;
            let file = match[1];
            if (file.startsWith("file:"))
                try {
                    file = fileURLToPath(file);
                }
                catch {
                    continue;
                }
            const sdk = relative(SDK_DIRECTORY, file);
            if (!isAbsolute(file) || !(sdk.startsWith("..") || isAbsolute(sdk)))
                continue;
            const inside = relative(this.directory, file);
            const shown = inside.startsWith("..") || isAbsolute(inside) ? basename(file) : inside;
            return `${shown.replaceAll("\\", "/")}:${match[2]}`;
        }
        return undefined;
    }
    /** Evaluate one binding, holding the events whose state it read. */
    #evaluate(evaluate, held) {
        const reads = new Map();
        const outer = this.#reads;
        const reader = this.#reader;
        this.#reads = reads;
        this.#reader = undefined;
        try {
            evaluate();
        }
        finally {
            this.#reads = outer;
            this.#reader = reader;
            for (const [event, hold] of held)
                if (!reads.has(event)) {
                    held.delete(event);
                    void hold.dispose();
                }
            for (const [event, why] of reads)
                if (!held.has(event))
                    held.set(event, this.hold(event, why));
        }
    }
    /** Coalesce: one evaluation pass per microtask turn, however many changes. */
    invalidate() {
        if (this.#scheduled)
            return;
        this.#scheduled = true;
        queueMicrotask(() => {
            this.#scheduled = false;
            for (const [evaluate, held] of [...this.#bindings]) {
                if (this.#bindings.get(evaluate) !== held)
                    continue;
                try {
                    this.#evaluate(evaluate, held);
                }
                catch (error) {
                    console.error(`[${this.manifest.id}] binding failed:`, error);
                }
            }
        });
    }
    /**
     * Host events with one `events.subscribe` per name, however many listeners.
     * Without the `events` capability the listener simply never fires.
     * `mirror`: the SDK's own copy of the state (`plugin.player.state` and
     * the like), not a listener of the plugin's (see {@link EventGroup}).
     */
    on(event, listener, 
    /** The capability the event needs; `null`: none. */
    needs = "events", mirror = false) {
        let entry = this.#events.get(event);
        const created = !entry;
        if (!entry) {
            entry = { listeners: new Set(), active: 0, whys: new Map() };
            this.#events.set(event, entry);
        }
        const group = entry;
        group.listeners.add(listener);
        const token = {};
        if (!mirror) {
            group.active++;
            group.whys.set(token, listenerWhy(this.callSite()));
        }
        if (created && (needs === null || this.can(needs))) {
            group.told = group.active > 0;
            const whys = group.told ? allWhys(group) : undefined;
            group.toldWhy = whys?.join("\n");
            group.off = this.host.events.subscribe(event, (data) => {
                for (const each of [...group.listeners])
                    each(data);
            }, !group.told, whys);
            this.starting(group.off, (error) => console.error(`[${this.manifest.id}] subscribe ${event}:`, error));
        }
        else
            this.#tell(event, group);
        let listening = true;
        return {
            dispose: async () => {
                if (!listening)
                    return;
                listening = false;
                group.listeners.delete(listener);
                if (!mirror)
                    group.active--;
                group.whys.delete(token);
                if (this.#events.get(event) !== group)
                    return;
                if (group.listeners.size)
                    return this.#tell(event, group);
                this.#events.delete(event);
                const off = await group.off?.catch(() => undefined);
                await off?.().catch(() => { });
            },
        };
    }
    /**
     * Count `event` as listened to while held: a watcher of a service's
     * state, a binding that reads it. Nothing when nothing subscribed to it
     * (no capability, or the service did not start).
     */
    hold(event, why) {
        const group = this.#events.get(event);
        if (!group)
            return { dispose: async () => { } };
        group.active++;
        const token = {};
        group.whys.set(token, why ?? listenerWhy(this.callSite()));
        this.#tell(event, group);
        let held = true;
        return {
            dispose: async () => {
                if (!held)
                    return;
                held = false;
                group.active--;
                group.whys.delete(token);
                if (this.#events.get(event) === group)
                    this.#tell(event, group);
            },
        };
    }
    /**
     * Tell the player when `group` changes between mirror and listener, and
     * why it listens when that changed.
     */
    #tell(event, group) {
        const off = group.off;
        if (!off)
            return;
        const active = group.active > 0;
        const whys = active ? allWhys(group) : undefined;
        const why = whys?.join("\n");
        if (group.told === active && group.toldWhy === why)
            return;
        group.told = active;
        group.toldWhy = why;
        // Part of activation while it runs: the player classifies the plugin
        // at its ready, which must see this mark. A failure is not one of
        // activation (an older player never refuses it).
        const told = off
            .then(() => this.host.events.listen(event, active, whys))
            .catch(() => { });
        this.starting(told, () => { });
    }
    /**
     * Why the plugin listens to each event it really listens to (mirrors
     * left out), as the player's log says it: for {@link TestHost} and
     * `vestra-plugin test`.
     */
    listening() {
        const result = new Map();
        for (const [event, group] of this.#events)
            if (group.active > 0)
                result.set(event, allWhys(group));
        return result;
    }
    /** Raw transport events the host pushes without a subscription. */
    raw(event, listener) {
        const off = this.#transport.on(event, listener);
        return { dispose: async () => off() };
    }
}
/** A plugin listener's reason: where it was added. */
const listenerWhy = (site) => () => {
    const at = site();
    return at ? `a listener added at ${at}` : "a listener";
};
/** Every active holder's reason in `group`, in the order they came, once each. */
function allWhys(group) {
    return [...new Set([...group.whys.values()].map((why) => why()))];
}
/** `plugin.app` from the start-up handshake (the runner's environment). */
export function appInfo(raw, system = () => Promise.reject(failure("unavailable", "no player to ask"))) {
    const info = (raw ?? {});
    const parts = (info.components ?? {});
    const text = (value) => (typeof value === "string" ? value : null);
    const libav = {};
    for (const [name, version] of Object.entries(parts.libav ?? {}))
        if (typeof version === "string")
            libav[name] = version;
    const app = {
        version: text(info.version) ?? "0.0.0",
        build: text(info.build),
        components: Object.freeze({
            mpv: text(parts.mpv),
            libav: Object.freeze(libav),
            slint: text(parts.slint) ?? "",
        }),
    };
    // A method, not data: kept out of comparisons and JSON of the facts.
    Object.defineProperty(app, "system", { value: system, enumerable: false });
    return Object.freeze(app);
}
