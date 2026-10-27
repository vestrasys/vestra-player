const BOOL = [true, false];
/** Value ranges, the same as the host's `effects.rs` (the host is authoritative). */
const RULES = {
    eq: {
        brightness: [-1, 1],
        contrast: [0, 2],
        saturation: [0, 2],
        gamma: [0.5, 2],
    },
    gblur: { sigma: [0, 10], steps: [3] },
    hue: { degrees: [-180, 180] },
    sharpen: { amount: [0, 2], size: ["small", "large"] },
    denoise: {
        lumaSpatial: [0, 20],
        chromaSpatial: [0, 20],
        lumaTemporal: [0, 20],
        chromaTemporal: [0, 20],
    },
    deblock: { strength: ["weak", "strong"] },
    deband: { threshold: [0.003, 0.5], range: [1, 64] },
    transform: { rotate: [0, 90, 180, 270], flipH: BOOL, flipV: BOOL },
    deinterlace: {
        when: ["interlaced", "all", "auto"],
        output: ["frame", "field"],
    },
    reframe: { mode: ["crop", "pad"], aspect: [0.25, 4], x: [-1, 1], y: [-1, 1] },
    frameBlend: { fps: [24, 120] },
    volume: { gainDb: [-60, 0] },
    equalizer: { frequency: [20, 20000], q: [1], gainDb: [-6, 6] },
    treble: { frequency: [3000], slope: [0.5], gainDb: [0, 6] },
    dynaudnorm: { enabled: BOOL, maxGain: [1, 4], peak: [0.5, 0.95] },
    channels: { mode: ["stereo", "left", "right", "mono", "swap"] },
};
const VIDEO = new Set([
    "eq",
    "gblur",
    "hue",
    "sharpen",
    "denoise",
    "deblock",
    "deband",
    "transform",
    "deinterlace",
    "reframe",
    "frameBlend",
]);
/**
 * Why `nodes` would be refused (`null` when valid): unknown kinds or fields,
 * values out of range, more than 64 nodes. Used by the test host.
 */
export function nodeProblem(stage, nodes) {
    if (nodes.length > 64)
        return "chain limit: 64 nodes";
    for (const node of nodes) {
        const rule = RULES[node.kind];
        if (!rule || VIDEO.has(node.kind) !== (stage === "video"))
            return `unknown ${stage} node ${node.kind}`;
        const fields = Object.keys(node).filter((key) => key !== "kind");
        for (const field of fields)
            if (!(field in rule))
                return `${node.kind}: unknown field ${field}`;
        for (const [field, allowed] of Object.entries(rule)) {
            const value = node[field];
            const ok = allowed.length === 2 &&
                typeof allowed[0] === "number" &&
                typeof allowed[1] === "number" &&
                typeof value === "number"
                ? Number.isFinite(value) && value >= allowed[0] && value <= allowed[1]
                : allowed.includes(value);
            if (!ok)
                return `${node.kind}.${field} out of range`;
        }
        if (node.kind === "deband" &&
            !Number.isInteger(node.range))
            return "deband.range must be an integer";
        if (node.kind === "frameBlend" &&
            !Number.isInteger(node.fps))
            return "frameBlend.fps must be an integer";
        if (node.kind === "deinterlace" &&
            node.when === "auto" &&
            node.output !== "field")
            return "deinterlace when auto outputs one frame per field: output must be field";
    }
    const deinterlace = nodes.filter((node) => node.kind === "deinterlace");
    if (deinterlace.length > 1 &&
        deinterlace.some((node) => node.when === "auto"))
        return "a chain with a deinterlace node when auto has no other deinterlace node";
    if (nodes.filter((node) => node.kind === "frameBlend").length > 1)
        return "only one frameBlend node across all plugins";
    return null;
}
export function processingClient(transport) {
    return {
        state: () => transport.request("processing.state"),
        /** Token is (stage, resourceId, revision). Fix it before sending. A dynamic
         * rejection consumes this token; never change its payload on retry. */
        replace: (change) => transport.request("processing.replace", change),
        operation: (token) => transport.request("processing.operation", token),
    };
}
