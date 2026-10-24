export function subtitleClient(transport) {
    return {
        operation: (operationId) => transport.request("subtitles.operation", { operationId }),
        state: () => transport.request("subtitles.state"),
        /** Property application is not a visible-frame or exact-font identity fence. */
        style: (options) => transport.request("subtitles.style", options),
        begin: (uploadId, generation, expectedBytes) => transport.request("subtitles.upload.begin", {
            uploadId,
            generation,
            expectedBytes,
        }),
        append: (uploadId, offset, base64) => transport.request("subtitles.upload.append", {
            uploadId,
            offset,
            base64,
        }),
        discard: (uploadId) => transport.request("subtitles.upload.discard", { uploadId }),
        /** Atomic registration plus selection. inputOrigin guards a translation's original content revision. */
        commit: (options) => transport.request("subtitles.commit", options),
        select: (target, expected, operationId, role = "primary") => transport.request("subtitles.select", {
            target,
            expected,
            operationId,
            // The default is left out: the request stays what older hosts accept.
            ...(role === "secondary" ? { role } : {}),
        }),
        remove: (resourceId, expected, operationId) => transport.request("subtitles.remove", {
            resourceId,
            expected,
            operationId,
        }),
    };
}
