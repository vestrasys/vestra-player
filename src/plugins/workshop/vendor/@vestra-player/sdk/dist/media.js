export function mediaClients(transport) {
    function capture(options) {
        return transport.request("snapshot.capture", options);
    }
    return {
        snapshot: {
            capture,
            state: (snapshotId) => transport.request("snapshot.state", { snapshotId }),
            release: (snapshotId, operationId) => transport.request("snapshot.release", {
                snapshotId,
                operationId,
            }),
        },
        jobs: {
            start: (options) => transport.request("jobs.start", options),
            state: (jobId) => transport.request("jobs.state", { jobId }),
            cancel: (jobId, operationId) => transport.request("jobs.cancel", { jobId, operationId }),
            operation: (operationId) => transport.request("jobs.operation", { operationId }),
            /** `jobs.start` with parts of remembered files instead of a snapshot. */
            startParts: (options) => transport.request("jobs.start", options),
            check: (parts, encoding) => transport.request("jobs.check", encoding === undefined ? { parts } : { parts, encoding }),
            /** `jobs.check` of a snapshot's clip (to `end`) instead of parts. */
            checkSnapshot: (snapshotId, end, encoding) => transport.request("jobs.check", encoding === undefined
                ? { snapshotId, end }
                : { snapshotId, end, encoding }),
        },
        mediaSources: {
            remember: (expected) => transport.request("media.sources.remember", expected === undefined ? {} : { expected }),
            get: (token) => transport.request("media.sources.get", {
                token,
            }),
            locate: (token) => transport.request("media.sources.locate", {
                token,
            }),
            forget: (token) => transport.request("media.sources.forget", { token }),
            list: () => transport.request("media.sources.list", {}),
            /** An `open` submit of the file, under the player service's operation id. */
            open: (options) => transport.request("media.sources.open", options),
        },
    };
}
