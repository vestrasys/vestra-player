/**
 * Source resolvers: page URLs to stream URLs, or local files to a stream
 * this plugin generates, for `player.open`.
 */
import { failure, quiet } from "./runtime.js";
import { playerOf } from "./player.js";
import { resolvedStream } from "./stream.js";
import { windowOf } from "./window.js";
const isFiles = (options) => "extensions" in options;
const sameTypes = (a, b) => a.length === b.length && a.every((type, i) => type === b[i]);
/**
 * One per plugin: the list comes with the `window.state` read before
 * activation (`[]` from older players) and is read again when the plugin
 * first uses it; with `events`, `resolvers.fileTypes` keeps it current.
 */
class FileTypesService {
    #runtime;
    types;
    #listeners = new Set();
    #started = false;
    constructor(runtime) {
        this.#runtime = runtime;
        this.types = Object.freeze([...(windowOf(runtime).fileTypes ?? [])]);
    }
    start() {
        if (this.#started)
            return;
        this.#started = true;
        this.#runtime.on("resolvers.fileTypes", (data) => {
            const types = data?.fileTypes;
            if (Array.isArray(types) && types.every((t) => typeof t === "string"))
                this.accept(types);
        });
        // What changed between the start-up read and the subscription.
        const window = windowOf(this.#runtime);
        void window.read().then(() => {
            if (window.fileTypes)
                this.accept(window.fileTypes, this.#runtime.can("events"));
        });
    }
    /** `notify`: false without `events`, whose listeners never fire. */
    accept(types, notify = true) {
        if (sameTypes(types, this.types))
            return;
        this.types = Object.freeze([...types]);
        if (notify)
            for (const listener of [...this.#listeners])
                listener([...types]);
        this.#runtime.invalidate();
    }
    watch(listener) {
        this.#listeners.add(listener);
        return { dispose: async () => void this.#listeners.delete(listener) };
    }
}
const fileTypesOf = (runtime) => runtime.service("fileTypes", () => new FileTypesService(runtime));
export const resolvers = (scope) => ({
    get fileTypes() {
        const service = fileTypesOf(scope.runtime);
        service.start();
        return service.types;
    },
    onFileTypesChange(listener) {
        const service = fileTypesOf(scope.runtime);
        service.start();
        const watch = service.watch((types) => scope.run(listener, types));
        return scope.add({
            dispose: async () => {
                scope.forget(watch);
                await watch.dispose();
            },
        });
    },
    add(options) {
        const runtime = scope.runtime;
        const registration = isFiles(options)
            ? runtime.host.sourceResolvers.register({ id: options.id, extensions: options.extensions }, async ({ url, signal }) => {
                const stream = await options.resolve(url, signal);
                if (stream?.kind !== "stream")
                    throw failure("invalid", 'a file resolver returns { kind: "stream", width, height, frame }');
                return resolvedStream(scope, playerOf(runtime), stream, signal);
            })
            : runtime.host.sourceResolvers.register({ id: options.id, schemes: options.schemes, hosts: options.hosts }, ({ url, signal }) => options.resolve(url, signal));
        let disposed = false;
        const handle = {
            ready: quiet(runtime
                .starting(registration, (error) => scope.report(error))
                .then(() => { })),
            async dispose() {
                if (disposed)
                    return;
                disposed = true;
                scope.forget(handle);
                const registered = await registration.catch(() => undefined);
                await registered?.dispose().catch(() => { });
            },
        };
        return scope.add(handle);
    },
});
