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
        },
    };
}
