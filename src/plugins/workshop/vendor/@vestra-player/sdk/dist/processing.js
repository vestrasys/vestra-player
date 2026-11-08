/**
 * Processing chains for the player's video and audio filters. Applied means
 * the backend accepted the chain, not that a frame with it is visible yet.
 */
import { isAbsolute } from "node:path";
/**
 * The player's bounds for the video primitives: `convolution` and `lut3d`
 * {@link VideoNode}s and the shader channel (`plugin.video.shaders()`). Values
 * beyond them fail with `invalid`; more shaders than `shaders` fail with
 * `limit`.
 *
 * @example Refuse a shader list the player would not take
 * The plugin checks a shader list against the player's limit before it
 * sets it, and says so on the OSD instead of waiting for a `limit` failure.
 * The limit counts every plugin's shaders, so the player still decides.
 * ```ts
 * import type { Plugin } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare const files: string[];
 * // ---cut---
 * import { VIDEO_PROCESSING_LIMITS } from "@vestra-player/sdk/testing";
 *
 * if (files.length > VIDEO_PROCESSING_LIMITS.shaders)
 *   void plugin.osd.show("Too many shaders");
 * else plugin.video.shaders().set(files);
 * ```
 *
 * @since 1.5.39
 * @category Testing
 */
export const VIDEO_PROCESSING_LIMITS = Object.freeze({
    /** Shaders across every plugin's `plugin.video.shaders()` handles (16). */
    shaders: 16,
    /** Bytes of one shader file (16 MiB). */
    shaderBytes: 16 * 1024 * 1024,
    /** Bytes of a shader handle's id (128). */
    shaderIdBytes: 128,
    /** Bytes of one `lut3d` `.cube` file (16 MiB). */
    lutBytes: 16 * 1024 * 1024,
    /** Largest `LUT_3D_SIZE` of a `lut3d` file, points per axis (65). */
    lutSize: 65,
    /** Bytes of a `lut3d` file path (4096). */
    lutPathBytes: 4096,
    /** Largest magnitude of a `convolution` matrix entry (1024). */
    matrixValue: 1024,
    /** Largest `convolution` `rdiv` (256). */
    rdiv: 256,
    /** Largest `convolution` `bias` (65535). */
    bias: 65535,
});
const BOOL = [true, false];
/** Value ranges, the same as the host's `src/host/src/playback/effects.rs` (the host is authoritative). */
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
    // Checked by `convolutionProblem` / `lutProblem` below.
    convolution: { planes: [] },
    lut3d: {
        file: [],
        interpolation: ["nearest", "trilinear", "tetrahedral", "pyramid", "prism"],
    },
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
    "convolution",
    "lut3d",
]);
/** `convolution` nodes: the player's rules (`effects.rs ConvolutionPlane`). */
function convolutionProblem(planes) {
    if (!Array.isArray(planes) || planes.length < 1 || planes.length > 4)
        return "convolution planes must have 1 to 4 entries";
    if (planes.every((plane) => plane === null))
        return "convolution needs at least one plane with a matrix";
    const limits = VIDEO_PROCESSING_LIMITS;
    for (const [index, plane] of planes.entries()) {
        if (plane === null)
            continue;
        if (typeof plane !== "object" || Array.isArray(plane))
            return `convolution plane ${index} must be an object or null`;
        const fields = Object.keys(plane);
        const unknown = fields.find((field) => !["mode", "matrix", "rdiv", "bias"].includes(field));
        if (unknown)
            return `convolution plane ${index}: unknown field ${unknown}`;
        const { mode, matrix, rdiv, bias } = plane;
        if (!["square", "row", "column"].includes(mode))
            return `convolution plane ${index}: mode must be square, row or column`;
        if (!Array.isArray(matrix))
            return `convolution plane ${index}: matrix must be an array of integers`;
        const sized = mode === "square"
            ? [9, 25, 49].includes(matrix.length)
            : matrix.length % 2 === 1 && matrix.length <= 49;
        if (!sized)
            return `convolution plane ${index}: a square matrix has 9, 25 or 49 entries, a row or column an odd number up to 49 (got ${matrix.length})`;
        if (matrix.some((value) => !Number.isInteger(value) || Math.abs(value) > limits.matrixValue))
            return `convolution plane ${index}: matrix entries must be integers in -${limits.matrixValue}..${limits.matrixValue}`;
        const outside = (value, max) => typeof value !== "number" ||
            !Number.isFinite(value) ||
            value < 0 ||
            value > max;
        if (outside(rdiv, limits.rdiv))
            return `convolution plane ${index} rdiv must be finite in 0..=${limits.rdiv}`;
        if (outside(bias, limits.bias))
            return `convolution plane ${index} bias must be finite in 0..=${limits.bias}`;
    }
    return null;
}
/**
 * Whether `text` has a control character as Rust's `char::is_control`
 * sees one (C0, DEL and C1: U+0000–U+001F, U+007F–U+009F).
 */
export function hasControl(text) {
    return [...text].some((c) => {
        const code = c.codePointAt(0);
        return code < 0x20 || (code >= 0x7f && code <= 0x9f);
    });
}
/**
 * The form of a `lut3d` path (the player's `effects.rs lut_path`); where
 * the file is and what it holds the player checks when the request arrives
 * (the test host too, `TestHost.fileRoots`).
 */
export function lutPathProblem(file) {
    const max = VIDEO_PROCESSING_LIMITS.lutPathBytes;
    if (typeof file !== "string" ||
        Buffer.byteLength(file) > max ||
        hasControl(file))
        return `lut3d file must be a path of at most ${max} bytes without control characters`;
    if (/[[\]]/.test(file))
        return "lut3d file path must not contain [ or ]";
    if (!isAbsolute(file))
        return "lut3d file must be an absolute path";
    if (!/\.cube$/i.test(file))
        return "lut3d file must be a .cube file";
    return null;
}
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
        if (node.kind === "convolution") {
            const problem = convolutionProblem(node.planes);
            if (problem)
                return problem;
            continue;
        }
        if (node.kind === "lut3d") {
            const problem = lutPathProblem(node.file);
            if (problem)
                return problem;
        }
        for (const [field, allowed] of Object.entries(rule)) {
            if (allowed.length === 0)
                continue;
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
