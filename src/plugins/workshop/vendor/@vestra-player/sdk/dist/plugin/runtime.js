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
    // Player 0.7.6: sent only to the plugin whose own keys changed.
    "commands.keys",
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
    "commands.keys": { name: "plugin.commands.keys", when: false },
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
/**
 * The player's answers to a declaration that refuse that one declaration
 * (a command's path, texts or count): during activation they skip it
 * instead of failing the plugin (`Runtime.declaring`, SDK 1.5.49).
 */
const DECLARATION_REFUSALS = new Set([
    "invalid",
    "conflict",
    "limit",
]);
/** The reasons `StopReason` names; anything else reads as `undefined`. */
const STOP_REASONS = new Set([
    "quit",
    "disable",
    "uninstall",
    "reload",
    "upgrade",
    "idle",
    "failed",
    "host-lost",
]);
/** A stop reason the runner passed on, if this SDK knows it. */
export function stopReason(value) {
    return typeof value === "string" && STOP_REASONS.has(value)
        ? value
        : undefined;
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
    /** `RuntimeOptions.strictDeclarations`. */
    #strict;
    /** Declarations skipped during activation: reported once it completes. */
    #refusals = [];
    #transport;
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
    #events = new Map();
    views = new Map();
    commands = new Map();
    services = new Map();
    /**
     * Set when the player stops the plugin (`PluginInstance.dispose`), before
     * the root scope is disposed: `DisposeInfo` and `plugin.stopReason`.
     */
    stopping;
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
    constructor(transport, manifest, directory, dataDirectory, app, options = {}) {
        this.#transport = transport;
        this.#strict = options.strictDeclarations ?? false;
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
    /**
     * A declaration the player checks on its own (`what`: "command x"),
     * started like `starting`. In the player's runner a refusal of it during
     * activation (`invalid`, `conflict`, `limit`) does not fail the plugin:
     * it is logged, skipped and reported to the scope once activation
     * completes, so an `onError` added anywhere in `activate` hears it (SDK
     * 1.5.49; VS Code logs a bad contribution and activates the extension).
     * Under `strictDeclarations` (TestHost) it fails activation as before.
     */
    declaring(work, what, report) {
        if (!this.#activating || this.#strict)
            return this.starting(work, report);
        const settled = work.then(() => { }, (error) => {
            if (!(error instanceof HostError) ||
                !DECLARATION_REFUSALS.has(error.code))
                throw error;
            console.error(`[${this.manifest.id}] ${what} skipped, the player refused it (${error.code}): ${error.message}`);
            this.#refusals.push(() => report(error));
        });
        // Observed now: `activated()` may come after `activate` awaits, and
        // its `allSettled` still sees the rejection.
        settled.catch(() => { });
        this.startup.add(settled);
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
        for (const report of this.#refusals.splice(0))
            report();
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
            if (outerReads)
                for (const [event, why] of last.reads)
                    if (!outerReads.has(event))
                        outerReads.set(event, why);
            if (outerDeps)
                merge(outerDeps, last.deps);
            return undefined;
        }
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
                    held.set(event, this.hold(event, why));
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
                console.error(`[${this.manifest.id}] binding failed:`, error);
            }
            finally {
                this.#pass = undefined;
            }
        }
        if (this.#skipped && !this.#safety && !this.#stopped) {
            this.#safety = setTimeout(() => {
                this.#safety = undefined;
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
