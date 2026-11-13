/** Menu commands: declaration, state sync (revisions hidden) and dispatch. */
import { Latest } from "./latest.js";
import { failure, quiet, uncertain } from "./errors.js";
import { checkBindings } from "./bindings.js";
import { BAR_BUTTON_ICONS, BAR_BUTTON_LIMITS, MENU_IDS, thenProblem, } from "../commands.js";
import { ImageHandle } from "./view/image.js";
import { isTextValue, textProblem } from "../i18n.js";
import { conditionJson, isCondition } from "./when.js";
import { shownText } from "./osd.js";
const value = (item) => typeof item === "function" ? item() : item;
/**
 * A state field given as a function: the command is evaluated in the update
 * passes. Constants and `plugin.when` conditions are not.
 */
const tracks = (spec) => Object.values(spec).some((field) => typeof field === "function");
/** A field's value, or `fallback` (and its condition noted) for a condition. */
function field(item, fallback, name, when) {
    if (!isCondition(item))
        return value(item);
    when[name] = conditionJson(item);
    return fallback;
}
/** How the player's log names a command: its title, else its id. */
const label = (options) => {
    const title = options.title ?? options.path?.at(-1);
    return `command "${typeof title === "string" ? title : options.id}"`;
};
/** Validate options and build the wire declaration. */
function declare(options) {
    const path = options.path ?? [];
    const title = options.title ?? path.at(-1);
    if (!title)
        throw failure("invalid", `command ${options.id}: title or path required`);
    if (path.length > 8)
        throw failure("invalid", `command ${options.id}: path has 1–8 segments`);
    const category = options.category;
    for (const segment of [
        title,
        ...path,
        ...(category === undefined ? [] : [category]),
    ])
        if (typeof segment !== "string") {
            const problem = isTextValue(segment)
                ? "$vestraList" in segment
                    ? "a menu text is a string or a text, not a list"
                    : textProblem(segment)
                : "title, path segments and category are strings or texts";
            if (problem)
                throw failure("invalid", `command ${options.id}: ${problem}`);
        }
    const first = path[0];
    const menu = typeof first === "string" && first.startsWith("@") ? first.slice(1) : null;
    if (menu !== null && !MENU_IDS.includes(menu))
        throw failure("invalid", `command ${options.id}: unknown menu id @${menu}; the built-in menus are ${MENU_IDS.map((id) => `@${id}`).join(", ")}`);
    if (menu !== null && path.length < 2)
        throw failure("invalid", `command ${options.id}: menu id @${menu} needs an item after it`);
    const order = options.order ?? 0;
    if (!Number.isInteger(order) || Math.abs(order) > 1_000_000)
        throw failure("invalid", `command ${options.id}: order must be an integer within ±1000000`);
    const bindings = [...(options.bindings ?? [])];
    const declaration = {
        title: title,
        path: path,
        order,
        group: options.group ?? "domain",
        radio: options.radio ?? false,
        bindings: checkBindings(bindings),
        focus: options.focus ?? null,
        repeat: options.repeat ?? false,
    };
    // Only when set: players before 0.5.10 refuse the field.
    if (options.button !== undefined)
        declaration.button = button(options.id, options.button, path.length > 0);
    // Only when set: players before 0.7.6 refuse the fields.
    if (category !== undefined)
        declaration.category = category;
    if (options.destructive === true)
        declaration.destructive = true;
    else if (options.destructive !== undefined && options.destructive !== false)
        throw failure("invalid", `command ${options.id}: destructive is a boolean`);
    if (options.home !== undefined) {
        if (typeof options.home !== "string")
            throw failure("invalid", `command ${options.id}: home is a command id`);
        declaration.home = options.home;
    }
    return declaration;
}
/** `CommandOptions.button`, checked as the player checks it, in wire form. */
function button(id, value, inMenu) {
    const fail = (problem) => failure("invalid", `command ${id}: ${problem}`);
    if (value === null || typeof value !== "object")
        throw fail("button is { icon, order }");
    if (!inMenu)
        throw fail("button needs a menu path: a bar button is also a menu item");
    const { icon, order } = value;
    if (!Number.isInteger(order) ||
        order < 0 ||
        order > BAR_BUTTON_LIMITS.maxOrder)
        throw fail(`button order must be an integer 0–${BAR_BUTTON_LIMITS.maxOrder}`);
    return { icon: iconJson(icon, fail, "button"), order };
}
/** An icon name of the player's set or an image, checked, in wire form. */
function iconJson(icon, fail, what) {
    if (icon instanceof ImageHandle) {
        if (icon.released)
            throw fail(`${what} image ${icon.path} was released with the scope that created it`);
        return icon.toJSON();
    }
    if (typeof icon !== "string" ||
        !BAR_BUTTON_ICONS.includes(icon))
        throw fail(`unknown ${what} icon ${JSON.stringify(icon)}: a name of @vestra/icons.slint (BAR_BUTTON_ICONS) or an image from plugin.ui.image`);
    return icon;
}
class CommandHandle {
    id;
    ready;
    #scope;
    #spec;
    #revision = 0;
    #last = "";
    #disposed = false;
    #registered = false;
    /** Its entry in `runtime.commands` (a later handle may own the id). */
    #run = () => { };
    /** The player refused the declaration: no state updates follow. */
    #refused = false;
    #pump;
    #label;
    /** Where the plugin added it, for the player's log. */
    #where;
    /**
     * Its place in the update passes: only while a state field is a function
     * (`tracks`); a command of constants is registered and left alone.
     */
    #binding;
    /** A `set()` of a command outside the passes is sent in a microtask. */
    #queued = false;
    /** Its place in the order of evaluation, should it bind later. */
    #slot;
    constructor(scope, options) {
        const runtime = scope.runtime;
        this.#where = runtime.callSite();
        if (runtime.commands.has(options.id))
            throw failure("conflict", `command ${options.id} is already declared`);
        this.id = options.id;
        this.#scope = scope;
        this.#label = label(options);
        this.#spec = {
            enabled: options.enabled ?? true,
            checked: options.checked ?? null,
            busy: options.busy ?? false,
            reason: options.reason ?? null,
        };
        const declaration = declare(options);
        // The host requires a strictly increasing revision per update; the
        // registration itself is revision 1, so every later state follows it.
        const previous = runtime.removals.get(this.id);
        this.#pump = new Latest(async ({ state, when }) => {
            if (this.#refused)
                return;
            const revision = ++this.#revision;
            if (revision === 1) {
                await previous;
                try {
                    await runtime.host.commands.register(this.id, 1, declaration, state, when);
                }
                catch (error) {
                    // A definite refusal: the player has no such command to update.
                    if (!uncertain(error))
                        this.#refused = true;
                    throw error;
                }
                this.#registered = true;
            }
            else
                await runtime.host.commands.update(this.id, revision, state, when);
        });
        const initial = this.#state();
        this.#last = JSON.stringify(initial);
        this.ready = quiet(runtime
            .declaring(this.#pump.push(initial), `command ${this.id}`, (error) => scope.report(error))
            .then(() => { }));
        this.#run = () => {
            if (!this.#disposed)
                scope.run(options.run);
        };
        runtime.commands.set(this.id, this.#run);
        this.#slot = runtime.passes.slot();
        if (tracks(this.#spec))
            this.#binding = runtime.bind(() => this.#sync(), this.#slot);
        scope.add(this);
    }
    /**
     * The state to send. A condition field is sent as a placeholder with the
     * condition in `when`: the player evaluates it. Each function field is
     * evaluated under its own name, so what it reads is reported as this
     * command's (`Runtime.reader`).
     */
    #state() {
        const runtime = this.#scope.runtime;
        const spec = this.#spec;
        const as = (name, read) => runtime.reader(`${this.#label} ${name}`, this.#where, read, name === "enabled" || name === "checked");
        // enabled and checked first: what they read is reported as theirs
        // (a condition can replace them), not as the reason's.
        const when = {};
        const enabled = as("enabled", () => field(spec.enabled, true, "enabled", when));
        const checked = as("checked", () => field(spec.checked, false, "checked", when));
        const reason = as("reason", () => value(spec.reason));
        if (reason !== null && typeof reason !== "string") {
            const problem = isTextValue(reason)
                ? textProblem(reason)
                : "reason is a string, a text or null";
            if (problem)
                throw failure("invalid", `command ${this.id}: ${problem}`);
        }
        const state = {
            enabled,
            checked,
            busy: as("busy", () => value(spec.busy)),
            reason,
        };
        return Object.keys(when).length ? { state, when } : { state };
    }
    #sync() {
        if (this.#disposed || this.#refused)
            return;
        const state = this.#state();
        const encoded = JSON.stringify(state);
        if (encoded === this.#last)
            return;
        this.#last = encoded;
        this.#pump.push(state).catch((error) => this.#scope.report(error));
    }
    /**
     * This command's state alone (before, a full update pass): a bound command is
     * evaluated in the next pass, a command of constants sends its new state
     * through its own `Latest` in a microtask (several `set()` calls in one turn
     * send one update). A function field binds it.
     */
    set(state) {
        Object.assign(this.#spec, state);
        if (this.#disposed)
            return;
        if (!this.#binding && tracks(this.#spec))
            this.#binding = this.#scope.runtime.bind(() => this.#sync(), this.#slot);
        if (this.#binding)
            return this.#binding.invalidate();
        if (this.#queued)
            return;
        this.#queued = true;
        queueMicrotask(() => {
            this.#queued = false;
            try {
                this.#sync();
            }
            catch (error) {
                this.#scope.report(error);
            }
        });
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#scope.forget(this);
        const runtime = this.#scope.runtime;
        if (runtime.commands.get(this.id) === this.#run)
            runtime.commands.delete(this.id);
        this.#pump.stop();
        // Removed from the menu once registered. Recorded before any await, so an
        // immediate re-add of the same id registers after this removal.
        const pending = runtime.removals;
        const removal = (async () => {
            await this.#binding?.dispose();
            await this.ready.catch(() => { });
            if (this.#registered)
                await runtime.host.commands.unregister(this.id).catch(() => { });
        })();
        pending.set(this.id, removal);
        await removal;
        if (pending.get(this.id) === removal)
            pending.delete(this.id);
    }
}
const NONE = Object.freeze([]);
const KEYS = "commands.keys";
/**
 * One per plugin: the SDK's copy of the key labels of the plugin's own
 * commands. Started by the first `keys` read or a `player.on("commands.keys")`: it
 * subscribes to `commands.keys` (a mirror, unless a listener or a binding holds it), then
 * asks once; the player pushes every later change. Answers and events carry the
 * binding table's revision, and an older one than what the copy has is dropped.
 */
class KeysService {
    #runtime;
    #keys = new Map();
    #revision = -1;
    #started = false;
    constructor(runtime) {
        this.#runtime = runtime;
    }
    start() {
        if (this.#started)
            return;
        this.#started = true;
        const runtime = this.#runtime;
        runtime.on(KEYS, (data) => this.#accept(data), null, true);
        // Subscribed first: a change after this answer comes as an event.
        runtime.host.commands.keys().then((answer) => this.#accept(answer), 
        // `unsupported`: no keys, as without a binding.
        () => { });
    }
    #accept(data) {
        const answer = data;
        const revision = answer?.revision;
        const keys = answer?.keys;
        if (typeof revision !== "number" || !keys || typeof keys !== "object")
            return;
        if (revision < this.#revision)
            return;
        this.#revision = revision;
        const next = new Map();
        for (const [id, labels] of Object.entries(keys))
            if (Array.isArray(labels) && labels.length)
                next.set(id, Object.freeze(labels.map(String)));
        const changed = [];
        for (const id of new Set([...this.#keys.keys(), ...next.keys()])) {
            const before = this.#keys.get(id) ?? NONE;
            const after = next.get(id) ?? NONE;
            if (before.length !== after.length ||
                before.some((label, at) => label !== after[at]))
                changed.push(id);
            // The same labels keep the same array.
            else if (after !== NONE)
                next.set(id, before);
        }
        this.#keys = next;
        if (!changed.length)
            return;
        this.#runtime.changed(KEYS, changed);
    }
    get(id) {
        return this.#keys.get(id) ?? NONE;
    }
}
export const keysOf = (runtime) => runtime.service("commands.keys", () => new KeysService(runtime));
export const commands = (scope) => ({
    add: (options) => new CommandHandle(scope, options),
    keys(id) {
        if (typeof id !== "string")
            throw failure("invalid", "keys takes a command id of this plugin");
        const service = keysOf(scope.runtime);
        service.start();
        scope.runtime.reading(KEYS);
        scope.runtime.depends(KEYS, id);
        return service.get(id);
    },
    list: async () => {
        return scope.runtime.host.commands.list();
    },
    async run(id, options) {
        if (typeof id !== "string" || id === "")
            throw failure("invalid", "run takes a command id: <plugin>/<id>");
        const { ticket, step, of, then } = (options ?? {});
        if (typeof ticket !== "string" || ticket === "")
            throw failure("ticket", "run needs the ticket of a user action");
        // As the player: both or neither, 1 <= step <= of <= 99.
        const whole = (value) => Number.isInteger(value) &&
            value >= 1 &&
            value <= 99;
        if ((step === undefined) !== (of === undefined) ||
            (step !== undefined && (!whole(step) || !whole(of) || step > of)))
            throw failure("invalid", "step and of go together, 1 <= step <= of <= 99");
        // As the player: the rest of the plan, 0–7 non-empty ids.
        const planned = then ?? undefined;
        if (planned !== undefined) {
            const problem = thenProblem(planned);
            if (problem)
                throw failure("invalid", problem);
        }
        return scope.runtime.host.commands.run({
            id,
            ticket,
            ...(step === undefined ? {} : { step }),
            ...(of === undefined ? {} : { of }),
            // An array, never called: not a thenable.
            // oxlint-disable-next-line unicorn/no-thenable
            ...(planned === undefined ? {} : { then: [...planned] }),
        });
    },
});
// ---------------------------------------------------------------- palette lead
/** `PaletteLeadOptions` checked and in wire form; the player checks lengths. */
function leadJson(options) {
    if (options === null || typeof options !== "object")
        throw failure("invalid", "lead takes { title, icon, ... }");
    const text = (value, what) => shownText(value, undefined, what);
    const out = {
        title: text(options.title, "title"),
        icon: iconJson(options.icon, (problem) => failure("invalid", problem), "lead"),
    };
    for (const key of ["placeholder", "empty", "note"])
        if (options[key] !== undefined)
            out[key] = text(options[key], key);
    const status = options.status;
    if (status === null)
        out.status = null;
    else if (status !== undefined) {
        if (typeof status !== "object")
            throw failure("invalid", "status is { text, note?, state? } or null");
        out.status = {
            text: text(status.text, "status.text"),
            ...(status.note === undefined
                ? {}
                : { note: text(status.note, "status.note") }),
            // Sent only when set (players before them refuse the fields); the
            // player checks the values.
            ...(status.strip === undefined
                ? {}
                : { strip: text(status.strip, "status.strip") }),
            ...(status.state === undefined ? {} : { state: status.state }),
        };
    }
    return out;
}
class LeadHandle {
    ready;
    #scope;
    #pump;
    #listeners = new Set();
    #event;
    #disposed = false;
    constructor(scope, options) {
        const first = leadJson(options);
        this.#scope = scope;
        const host = scope.runtime.host.palette;
        this.#pump = new Latest((value) => host.set(value));
        // Not an activation step: a refusal (a development load is not built
        // in) reaches `ready` and onError, and the rest of the plugin runs.
        this.ready = quiet(this.#pump.push(first).then(() => { }));
        this.ready.catch((error) => scope.report(error));
        // Sent to the lead's owner unasked, like `commands.execute`.
        this.#event = scope.runtime.raw("palette.lead", (data) => {
            const raw = (data ?? {});
            const run = {
                query: String(raw.query ?? ""),
                keep: raw.keep === true,
                ticket: String(raw.ticket ?? ""),
                status: raw.status === true,
            };
            for (const listener of [...this.#listeners])
                scope.run(listener, run);
        });
        scope.add(this);
    }
    async update(options) {
        if (this.#disposed)
            return;
        await this.#pump.push(leadJson(options));
    }
    onRun(listener) {
        this.#listeners.add(listener);
        return { dispose: async () => void this.#listeners.delete(listener) };
    }
    async done(ticket) {
        if (typeof ticket !== "string" || ticket === "")
            throw failure("invalid", "done takes a ticket");
        await this.#scope.runtime.host.palette.done(ticket);
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#scope.forget(this);
        this.#listeners.clear();
        await this.#event.dispose();
        this.#pump.stop();
        if (await this.ready.then(() => true, () => false))
            await this.#scope.runtime.host.palette.clear().catch(() => { });
    }
}
export const palette = (scope) => ({
    lead: (options) => new LeadHandle(scope, options),
});
