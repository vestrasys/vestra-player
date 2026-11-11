export function sourceClient(transport) {
    let next = 0;
    return {
        state: () => transport.request("source.state"),
        /** `signal`: aborted while the submit still waits to be sent, it is not sent. */
        submit: (intent, operationId, signal) => {
            operationId ??= next + 1;
            if (!Number.isSafeInteger(operationId) || operationId <= 0) {
                return Promise.reject(Object.assign(new TypeError("operationId must be a positive safe integer"), { code: "invalid" }));
            }
            next = Math.max(next, operationId);
            return transport.request("source.submit", { operationId, intent }, { signal });
        },
        operation: (operationId) => transport.request("source.operation", { operationId }),
    };
}
