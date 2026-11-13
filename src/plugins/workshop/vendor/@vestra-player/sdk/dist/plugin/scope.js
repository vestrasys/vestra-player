import { failure, toError } from "./errors.js";
import { commands, keysOf, palette } from "./commands.js";
import { WHEN } from "./when.js";
import { dialogs } from "./services/dialogs.js";
import { ui } from "./view/image.js";
import { panels } from "./view/panel.js";
import { windows } from "./view/window.js";
import { timeline, timelineOf } from "./timeline.js";
import { drops } from "./drops.js";
import { appearance, appearanceOf } from "./appearance.js";
import { settings } from "./settings.js";
import { i18n } from "./language.js";
import { player } from "./player.js";
import { audioFilters, videoFilters } from "./filters.js";
import { videoRender, videoView } from "./view.js";
import { videoShaders } from "./shaders.js";
import { subtitles } from "./subtitles.js";
import { mediaCapture } from "./media.js";
import { resolvers } from "./resolvers.js";
import { packages } from "./packages.js";
import { osd } from "./osd.js";
import { mainWindow } from "./window.js";
import { bindingsApi } from "./bindings.js";
import { trackedState } from "./state.js";
import { wire } from "./wire-facade.js";
import { mpv } from "./mpv.js";
import { clipboard, host, power, rendering, secrets, storage, system, tools, videoMotion, videoUpscaling, } from "./services.js";
/**
 * The SDK's state copies that a player event keeps current, by event: a
 * `player.on` listener of the event starts its copy first, so the copy
 * already holds the event's data when the listener runs.
 */
const COPIES = {
    "commands.keys": (runtime) => keysOf(runtime).start(),
    "timeline.changed": (runtime) => timelineOf(runtime).start(),
    "appearance.changed": (runtime) => appearanceOf(runtime).start(),
};
export class ScopeImpl {
    runtime;
    #parent;
    #items = [];
    #errors = new Set();
    #disposed = false;
    constructor(runtime, parent) {
        this.runtime = runtime;
        this.#parent = parent;
    }
    get id() {
        return this.runtime.manifest.id;
    }
    get dataDirectory() {
        const directory = this.runtime.dataDirectory;
        if (!directory)
            throw failure("unavailable", "the player provided no data directory");
        return directory;
    }
    get stopReason() {
        return this.runtime.stopping?.reason;
    }
    get disposed() {
        return this.#disposed || (this.#parent?.disposed ?? false);
    }
    /**
     * Start the SDK's copy that `event` keeps current (`plugin.commands.keys`,
     * `plugin.timeline`, `plugin.appearance`), for `player.on`; nothing for
     * an event without one.
     */
    followCopy(event) {
        if (Object.hasOwn(COPIES, event))
            COPIES[event](this.runtime);
    }
    commands = commands(this);
    when = WHEN;
    windows = windows(this);
    panels = panels(this);
    dialogs = dialogs(this);
    player = player(this);
    video = {
        ...videoFilters(this),
        view: () => videoView(this),
        render: () => videoRender(this),
        shaders: (id) => videoShaders(this, id),
        upscaling: videoUpscaling(this),
        motion: (engine) => videoMotion(this, engine),
    };
    audio = audioFilters(this);
    subtitles = subtitles(this);
    media = mediaCapture(this);
    resolvers = resolvers(this);
    packages = packages(this);
    osd = osd(this);
    palette = palette(this);
    host = host(this);
    clipboard = clipboard(this);
    power = power(this);
    tools = tools(this);
    storage = storage(this);
    system = system(this);
    secrets = secrets(this);
    mpv = mpv(this);
    rendering = rendering(this);
    window = mainWindow(this);
    bindings = bindingsApi(this);
    ui = ui(this);
    timeline = timeline(this);
    drops = drops(this);
    appearance = appearance(this);
    settings = settings(this);
    i18n = i18n(this);
    wire = wire(this);
    get app() {
        return this.runtime.app;
    }
    scope() {
        return this.add(new ScopeImpl(this.runtime, this));
    }
    add(item) {
        if (this.#disposed)
            void item.dispose().catch(() => { });
        else
            this.#items.push(item);
        return item;
    }
    /** Handles call this when disposed directly, so the scope forgets them. */
    forget(item) {
        this.#items = this.#items.filter((each) => each !== item);
    }
    onDispose(cleanup) {
        let done = false;
        const item = {
            dispose: async () => {
                if (done)
                    return;
                done = true;
                this.forget(item);
                const stopping = this.runtime.stopping;
                await cleanup(Object.freeze(stopping
                    ? stopping.reason === undefined
                        ? { stopping: true }
                        : { stopping: true, reason: stopping.reason }
                    : { stopping: false }));
            },
        };
        return this.add(item);
    }
    onError(listener) {
        this.#errors.add(listener);
        return this.onDispose(() => {
            this.#errors.delete(listener);
        });
    }
    report(error) {
        const coded = toError(error);
        if (this.#errors.size)
            for (const listener of this.#errors) {
                try {
                    listener(coded);
                }
                catch (nested) {
                    console.error(`[${this.id}]`, nested);
                }
            }
        else if (this.#parent)
            this.#parent.report(coded);
        else
            console.error(`[${this.id}]`, coded);
    }
    /** Invoke plugin code on its behalf: report failures, then refresh bindings. */
    run(handler, ...args) {
        if (this.disposed)
            return;
        try {
            const result = handler(...args);
            if (result instanceof Promise)
                result
                    .catch((error) => this.report(error))
                    .finally(() => this.update());
        }
        catch (error) {
            this.report(error);
        }
        this.update();
    }
    /**
     * Run a listener of a state change (`player.onChange`) like `run`, except
     * that the full pass after it is left out when it returned synchronously
     * and read only fields of the state that `change` left alone: its result
     * then depends on nothing new. The precise pass for the change runs
     * anyway, and a full pass follows within `SAFETY_PASS_MS`.
     */
    runOn(change, handler, ...args) {
        if (this.disposed)
            return;
        let full = true;
        try {
            const { value, deps } = this.runtime.observe(() => handler(...args));
            if (value instanceof Promise)
                value.catch((error) => this.report(error)).finally(() => this.update());
            else
                full = this.runtime.needsFull(deps, change);
        }
        catch (error) {
            this.report(error);
        }
        if (full)
            this.update();
    }
    /**
     * Run a listener that pushes its own values (`player.on`) like `run`, except
     * that no full pass follows it: what it writes to `plugin.state` runs the
     * passes for its readers, and for anything else the plugin calls `update()`.
     * Its reads are not counted as a binding's.
     */
    runQuiet(handler, ...args) {
        if (this.disposed)
            return;
        try {
            const { value } = this.runtime.observe(() => handler(...args));
            if (value instanceof Promise)
                value.catch((error) => this.report(error));
        }
        catch (error) {
            this.report(error);
        }
    }
    update() {
        this.runtime.invalidate();
    }
    state(initial) {
        return trackedState(this, initial);
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#parent?.forget(this);
        for (const item of this.#items.splice(0).reverse()) {
            try {
                await item.dispose();
            }
            catch (error) {
                console.error(`[${this.id}] dispose failed:`, error);
            }
        }
        this.#errors.clear();
    }
}
