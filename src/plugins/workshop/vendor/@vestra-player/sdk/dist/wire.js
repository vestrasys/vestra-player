/**
 * Wire clients for the `vestra/1` JSON-RPC protocol. Internal to
 * SDK v2 (`sdk/plugin/`): plugins never receive these objects. Each method maps
 * one call to one request and adds nothing; revisions, operation ids,
 * conditional stamps and recovery live in `sdk/plugin/`.
 */
import { commandClient } from "./commands.js";
import { processingClient } from "./processing.js";
import { subtitleClient } from "./subtitles.js";
import { mediaClients } from "./media.js";
import { producerClient } from "./producer.js";
import { sourceClient } from "./source.js";
import { sourceResolverClient } from "./source-resolvers.js";
/** Working contract. Trusted plugin runtime, not a permissions sandbox. */
export const API_VERSION = "vestra/1";
/**
 * A definite refusal from the player (or the plugin runner), with a stable
 * `code`. Plugin code sees it as a `PluginError` (an `Error` with `code`);
 * branch on `code`, never on `message`.
 *
 * In tests, throw one from a {@link TestHost.handle} override to simulate the
 * player refusing a request: the SDK treats it exactly like a real refusal
 * (the call rejects with that `code`, nothing is retried). Throwing a plain
 * `Error` instead simulates a lost reply, which makes the SDK run its
 * recovery path (asking the host about the operation again).
 *
 * Common codes: `stale` (the condition no longer holds; nothing was done),
 * `invalid` (bad argument), `missing` (unknown target), `builtin-only`
 * (a built-in-only request from another plugin), `busy`, `conflict`, `unavailable`, `unsupported`
 * (refused, not executed), `backend` (load or parse failure), `failed`,
 * `timeout`, `aborted`, `limit` (a player limit such as the command count),
 * `disabled` (the plugin is stopping).
 *
 * @example Simulate the player refusing a clipboard write in a test
 * The test proves the plugin sees a refused write as an error with code
 * `failed`. `host.handle` replaces the test host's handler for one request
 * method; the `HostError` it throws travels back like the player's own
 * refusal, so the call rejects with that `code` and is not retried.
 * `host.reset` puts the default handler back for later steps.
 * ```ts
 * import assert from "node:assert/strict";
 * import { HostError, startPlugin } from "@vestra-player/sdk/testing";
 * import activate from "../main.ts";
 *
 * const { host, plugin, stop } = await startPlugin(activate, {
 *   directory: new URL("..", import.meta.url),
 * });
 * host.handle("clipboard.writeText", () => {
 *   throw new HostError("failed", "the system refused the write");
 * });
 * await assert.rejects(plugin.clipboard.writeText("x"), { code: "failed" });
 * host.reset("clipboard.writeText"); // writes succeed again
 * await stop();
 * ```
 *
 * @remarks
 * A definite refusal, as opposed to a lost reply: the SDK does not retry
 * it, and the plugin sees an `Error` with `code`.
 *
 * @since 1.0.0
 * @category Testing
 */
export class HostError extends Error {
    /** The stable error code, e.g. `"invalid"`, `"stale"`, `"unavailable"`. */
    code;
    /**
     * A refusal with this code, as the player would send it.
     *
     * @param code - The stable error code the caller will see as `error.code`.
     * @param message - Human-readable detail (not for branching on).
     */
    constructor(code, message) {
        super(message);
        this.code = code;
        this.name = "HostError";
    }
}
export function createWire(transport) {
    const subscriptions = new Map();
    return {
        source: sourceClient(transport),
        sourceResolvers: sourceResolverClient(transport),
        producer: producerClient(transport),
        subtitles: subtitleClient(transport),
        ...mediaClients(transport),
        processing: processingClient(transport),
        commands: commandClient(transport),
        /** The command palette's lead row (`palette.lead`). */
        palette: {
            set: (options) => transport.request("palette.lead.set", options),
            clear: () => transport.request("palette.lead.clear", {}),
            done: (ticket) => transport.request("palette.lead.done", { ticket }),
        },
        dialogs: {
            open: (options) => transport.request("dialogs.open", options),
            /** Message and detail are templates formatted with `args`. */
            confirm: (options) => transport.request("dialogs.confirm", options),
        },
        /** The main window: its state, size, level, pointer hiding and picture-in-picture. */
        window: {
            state: () => transport.request("window.state"),
            setVideoArea: (width, height) => transport.request("window.setVideoArea", { width, height }),
            setAlwaysOnTop: (onTop) => transport.request("window.setAlwaysOnTop", { onTop }),
            setCursorAutoHide: (mode) => transport.request("window.setCursorAutoHide", { mode }),
            setPictureInPicture: (on) => transport.request("window.setPictureInPicture", { on }),
        },
        /** Output devices (mpv ids) and the one to play on. */
        audioOutput: {
            devices: () => transport.request("player.audioDevices"),
            set: (id) => transport.request("player.setAudioDevice", { id }),
        },
        /** The binding table: list, user overlay. */
        bindings: {
            list: () => transport.request("bindings.list"),
            set: (plugin, command, inputs) => transport.request("bindings.set", { plugin, command, inputs }),
            reset: (ref) => transport.request("bindings.reset", ref ?? {}),
        },
        /** File drops: providers by extension, panel listeners. */
        drops: {
            provide: (id, extensions) => transport.request("drops.provide", { id, extensions }),
            withdraw: (id) => transport.request("drops.withdraw", { id }),
            listen: (listen) => transport.request("drops.listen", { listen }),
        },
        /** The palette: read it, change it. */
        appearance: {
            set: (theme) => transport.request("appearance.set", { theme }),
            /** The interface language for the next start. */
            setLanguage: (language) => transport.request("appearance.setLanguage", { language }),
        },
        /** Settings pages in the player's options window. */
        settings: {
            register: (options) => transport.request("settings.register", options),
            set: (id, property, write) => transport.request("settings.set", { id, property, ...write }),
            unregister: (id) => transport.request("settings.unregister", { id }),
            show: (id) => transport.request("settings.show", id === undefined ? {} : { id }),
        },
        /** The display layer (`plugin.video.view()`); the host checks ownership. */
        video: {
            viewSet: (values) => transport.request("video.view.set", {
                values,
            }),
            viewRelease: () => transport.request("video.view.release"),
            /** The GPU renderer's options (`plugin.video.render()`). */
            renderSet: (values) => transport.request("video.render.set", { values }),
            renderRelease: () => transport.request("video.render.release"),
            /**
             * One shader channel handle's list (`plugin.video.shaders(id)`); `[]`
             * removes it.
             */
            shadersSet: (id, paths) => transport.request("video.shaders.set", { id, paths }),
            /** The GPU's video upscaler (`plugin.video.upscaling`). */
            upscalingState: () => transport.request("video.upscaling.state"),
            upscalingSet: (enabled) => transport.request("video.upscaling.set", {
                enabled,
            }),
            /** Motion smoothing (`plugin.video.motion`). */
            motionState: (engine) => transport.request("video.motion.state", { engine }),
            motionSet: (engine, mode) => transport.request("video.motion.set", {
                engine,
                mode,
            }),
        },
        /** Mark layers on the seek bar (`plugin.timeline`). */
        timeline: {
            marks: () => transport.request("timeline.marks"),
            set: (marks, sourceId) => transport.request("timeline.set", {
                marks,
                ...(sourceId === undefined ? {} : { sourceId }),
            }),
            /** Replace this layer's ranges. */
            setRanges: (ranges, sourceId) => transport.request("timeline.setRanges", {
                ranges,
                ...(sourceId === undefined ? {} : { sourceId }),
            }),
        },
        /** Chapters and the A-B loop of the current media. */
        marks: {
            chapters: () => transport.request("player.chapters"),
            setLoop: (loop, expected) => transport.request("player.setLoop", {
                loop,
                ...(expected ? { expected } : {}),
            }),
        },
        playback: {
            /** Shared device output. Does not advance source intent. */
            set: (property, value) => transport.request("playback.set", { property, value }),
            stats: () => transport.request("playback.stats"),
        },
        /** External audio files and the selected audio track of the current media. */
        audio: {
            add: (path, select, expected) => transport.request("audio.addTrack", {
                path,
                select,
                ...(expected ? { expected } : {}),
            }),
            select: (id, expected) => transport.request("audio.selectTrack", {
                id,
                ...(expected ? { expected } : {}),
            }),
        },
        /** The core's own playback actions; resolves when the action finished. */
        host: {
            run: (action, seconds) => transport.request("host.run", seconds === undefined ? { action } : { action, seconds }),
        },
        step: {
            step: (unit, direction, expected) => transport.request("player.step", {
                unit,
                direction,
                ...(expected ? { expected } : {}),
            }),
            setStepSeek: (seconds, keyframes) => transport.request("player.setStepSeek", keyframes === undefined ? { seconds } : { seconds, keyframes }),
        },
        clipboard: {
            readText: () => transport.request("clipboard.readText"),
            writeText: (text) => transport.request("clipboard.writeText", { text }),
            writeImage: (path) => transport.request("clipboard.writeImage", { path }),
        },
        /** Files, folders and links shown or opened by the player. */
        system: {
            reveal: (paths) => transport.request("system.reveal", { paths }),
            openPath: (path) => transport.request("system.openPath", { path }),
            openUrl: (url) => transport.request("system.openUrl", { url }),
        },
        /**
         * The mpv pass-through (`mpv`): commands and properties as mpv names
         * them. The observed values arrive as `mpv.property` events.
         */
        mpv: {
            command: (args) => transport.request("mpv.command", { args }),
            get: (name) => transport.request("mpv.get", { name }),
            set: (name, value) => transport.request("mpv.set", { name, value }),
            observeProperty: (name) => transport.request("mpv.observe", { name }),
            unobserveProperty: (name) => transport.request("mpv.unobserve", { name }),
        },
        /**
         * Trigger conditions (1.10.0, `plugin.trigger`): `set` hands one over
         * (or replaces it), `off` removes it, `declare` names what `activate()`
         * declared. Each `set` and `off` answers the condition's sequence
         * number; the flips arrive as `triggers.fired` events.
         */
        triggers: {
            set: (trigger, when) => transport.request("triggers.set", { trigger, when }),
            off: (trigger, unhandled = false) => transport.request("triggers.off", unhandled ? { trigger, unhandled } : { trigger }),
            declare: (names, unnamed) => transport.request("triggers.declare", { names, unnamed }),
        },
        /** The system's secret store, per plugin (`secrets`). */
        secrets: {
            get: (name) => transport.request("secrets.get", { name }),
            set: (name, value) => transport.request("secrets.set", { name, value }),
            delete: (name) => transport.request("secrets.delete", { name }),
        },
        /**
         * JSON values the player keeps per plugin until it quits. `get` answers
         * `{}` for a missing key.
         */
        storageSession: {
            get: (key) => transport.request("storage.session.get", { key }),
            set: (key, value) => transport.request("storage.session.set", { key, value }),
            delete: (key) => transport.request("storage.session.delete", { key }),
            keys: () => transport.request("storage.session.keys"),
        },
        app: {
            system: () => transport.request("app.system"),
            openWindow: (paths) => transport.request("app.openWindow", { paths }),
        },
        /**
         * The rendering choice: `state`, `setPreference` and `restart`.
         */
        rendering: {
            state: () => transport.request("rendering.state"),
            setPreference: (preference) => transport.request("rendering.setPreference", {
                preference,
            }),
            restart: () => transport.request("rendering.restart"),
        },
        power: {
            preventSleep: (mode) => transport.request("power.preventSleep", { mode }),
        },
        /** Shared next/previous; the host routes `sequence.step` events to the provider. */
        sequence: {
            set: (flags) => transport.request("sequence.set", { flags }),
            step: (direction) => transport.request("sequence.step", { direction }),
        },
        /** `text` is a template in the plugin's catalog, formatted with `args`. */
        osd: {
            show: (text, args, actions) => transport.request("osd.show", {
                text,
                ...(args === undefined ? {} : { args }),
                // Sent only when given.
                ...(actions === undefined ? {} : { actions }),
            }),
            panel: (text, args, maxColumns) => transport.request("osd.panel", {
                text,
                ...(args === undefined ? {} : { args }),
                ...(maxColumns === undefined ? {} : { maxColumns }),
            }),
        },
        layout: {
            state: () => transport.request("layout.state"),
            set: (options) => transport.request("layout.set", options),
        },
        events: {
            /**
             * One `events.subscribe` per name however many listeners. `mirror`: only
             * the SDK keeps a copy of the state; `listen` changes it.
             */
            async subscribe(event, listener, mirror = false, 
            /** What listens (the player logs it). */
            why) {
                const off = transport.on(event, listener);
                let subscription = subscriptions.get(event);
                if (!subscription) {
                    subscription = {
                        count: 0,
                        ready: transport.request("events.subscribe", mirror
                            ? { event, mirror: true }
                            : why?.length
                                ? { event, why }
                                : { event }),
                    };
                    subscriptions.set(event, subscription);
                }
                subscription.count++;
                try {
                    await subscription.ready;
                }
                catch (error) {
                    off();
                    subscriptions.delete(event);
                    throw error;
                }
                let active = true;
                return async () => {
                    if (!active)
                        return;
                    active = false;
                    off();
                    if (--subscription.count === 0) {
                        subscriptions.delete(event);
                        await transport.request("events.unsubscribe", { event });
                    }
                };
            },
            /**
             * Tell the player whether a subscribed event has a real listener
             * (`active`) or only the SDK's copy of the state: `events.subscribe`
             * again, which replaces the mark.
             */
            listen: (event, active, why) => transport.request("events.subscribe", active
                ? why?.length
                    ? { event, why }
                    : { event }
                : { event, mirror: true }),
        },
        ui: {
            /** The remote images a Markdown document shows. */
            markdownScan: (text, base) => transport.request("ui.markdownScan", {
                text,
                base,
            }),
            mount: (options) => transport.request("ui.mount", options),
            // `slot`: which of the plugin's panels (one per slot).
            setMount: (property, write, slot) => transport.request("ui.setMount", {
                property,
                ...write,
                ...(slot ? { slot } : {}),
            }),
            interaction: (locks, slot) => transport.request("ui.interaction", {
                ...locks,
                ...(slot ? { slot } : {}),
            }),
            unmount: (slot) => transport.request("ui.unmount", slot ? { slot } : undefined),
            menu: (items, x, y, slot) => transport.request("ui.menu", {
                items,
                x,
                y,
                ...(slot ? { slot } : {}),
            }),
            open: (id, file, properties = {}, callbacks = []) => transport.request("ui.open", { id, file, properties, callbacks }),
            set: (id, property, write) => transport.request("ui.set", { id, property, ...write }),
            show: (id, visible) => transport.request("ui.show", { id, visible }),
            close: (id) => transport.request("ui.close", { id }),
        },
        /** Executes trusted package code. Validation checks production JS and Slint. */
        packages: {
            validate: (directory, apiVersion = API_VERSION) => transport.request("packages.validate", {
                directory,
                apiVersion,
            }),
            /** A `.vxp` file. */
            validateArchive: (file) => transport.request("packages.validateArchive", {
                file,
            }),
            installed: (id) => transport.request("packages.installed", { id }),
            /**
             * `source` and `replace` are sent only when given; `enabled` only when
             * given.
             */
            install: (token, expectedRevision, requestId, options = {}) => transport.request("packages.install", {
                token,
                expectedRevision,
                requestId,
                ...(options.source === undefined ? {} : { source: options.source }),
                ...(options.replace ? { replace: true } : {}),
                ...(options.enabled === undefined
                    ? {}
                    : { enabled: options.enabled }),
            }),
            operation: (operationId) => transport.request("packages.operation", {
                operationId,
            }),
            cancel: (operationId) => transport.request("packages.cancel", { operationId }),
            list: () => transport.request("packages.list"),
            setEnabled: (id, enabled) => transport.request("packages.setEnabled", { id, enabled }),
            reload: (id) => transport.request("packages.reload", { id }),
            uninstall: (id, removeData, expectedRevision, requestId) => transport.request("packages.uninstall", {
                id,
                removeData,
                expectedRevision,
                requestId,
            }),
        },
    };
}
