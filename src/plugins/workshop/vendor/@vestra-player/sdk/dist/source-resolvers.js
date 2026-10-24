const isProducer = (result) => result?.kind === "stream";
/** The words of a failure, as the user will read them. */
const reason = (error) => (error instanceof Error ? error.message : String(error)).slice(0, 2048);
export function sourceResolverClient(transport) {
    return {
        /** Requires source.resolve. One registered host/scheme pair, and one file
         * extension, has one owner (another plugin: `conflict`, reason
         * `resolver-taken`). Resolution shares source.submit arbitration and the
         * original operation ID. This backend supports paired streams with
         * identical header maps; different maps fail explicitly. No header is
         * silently discarded or made global. A failure is sent as its message. */
        async register(declaration, handler) {
            const active = new Map();
            let disposed = false;
            const offRequest = transport.on("source.resolveRequest", (value) => {
                const request = value;
                if (request.resolverId !== declaration.id || disposed)
                    return;
                const controller = new AbortController();
                active.set(request.requestId, controller);
                const live = () => !controller.signal.aborted && !disposed;
                void (async () => {
                    let result;
                    try {
                        result = await handler({
                            url: request.url,
                            signal: controller.signal,
                        });
                    }
                    catch (error) {
                        if (live())
                            await transport
                                .request("sourceResolvers.respond", {
                                requestId: request.requestId,
                                error: reason(error),
                            })
                                .catch(() => { });
                        active.delete(request.requestId);
                        return;
                    }
                    try {
                        if (!isProducer(result)) {
                            if (live())
                                await transport.request("sourceResolvers.respond", {
                                    requestId: request.requestId,
                                    result,
                                });
                            return;
                        }
                        if (!live())
                            return void (await result.abandoned());
                        let reply;
                        try {
                            // Answered: the host no longer cancels this request.
                            active.delete(request.requestId);
                            reply = await transport.request("sourceResolvers.respond", {
                                requestId: request.requestId,
                                result: { kind: "stream", producerId: result.producerId },
                            });
                        }
                        catch (error) {
                            await result.abandoned();
                            throw error;
                        }
                        await result.accepted(reply);
                    }
                    finally {
                        active.delete(request.requestId);
                    }
                })().catch(() => { });
            });
            const offCancel = transport.on("source.resolveCancel", (value) => {
                const request = value;
                if (request.resolverId === declaration.id) {
                    active.get(request.requestId)?.abort();
                    active.delete(request.requestId);
                }
            });
            try {
                await transport.request("sourceResolvers.register", declaration);
            }
            catch (error) {
                offRequest();
                offCancel();
                throw error;
            }
            return {
                async dispose() {
                    if (disposed)
                        return;
                    disposed = true;
                    for (const controller of active.values())
                        controller.abort();
                    active.clear();
                    offRequest();
                    offCancel();
                    await transport.request("sourceResolvers.unregister", {
                        id: declaration.id,
                    });
                },
            };
        },
    };
}
