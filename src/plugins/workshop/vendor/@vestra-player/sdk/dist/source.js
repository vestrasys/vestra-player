export function sourceClient(transport) {
    let next = 0;
    return {
        state: () => transport.request("source.state"),
        submit: (intent, operationId) => {
            operationId ??= next + 1;
            if (!Number.isSafeInteger(operationId) || operationId <= 0) {
                return Promise.reject(Object.assign(new TypeError("operationId must be a positive safe integer"), { code: "invalid" }));
            }
            next = Math.max(next, operationId);
            return transport.request("source.submit", {
                operationId,
                intent,
            });
        },
        operation: (operationId) => transport.request("source.operation", { operationId }),
    };
}
