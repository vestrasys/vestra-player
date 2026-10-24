/**
 * Source resolvers: page URLs to stream URLs, or local files to a stream
 * this plugin generates, for `player.open`.
 */
import { failure, quiet } from "./runtime.js";
import { playerOf } from "./player.js";
import { resolvedStream } from "./stream.js";
const isFiles = (options) => "extensions" in options;
export const resolvers = (scope) => ({
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
