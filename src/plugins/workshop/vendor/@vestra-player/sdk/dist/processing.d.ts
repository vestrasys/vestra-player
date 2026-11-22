import type { Transport } from "./wire.ts";
/**
 * One node of a {@link VideoFilter} chain, by `kind`. Values outside the
 * ranges below fail with `invalid`. Fields marked "live" change in place
 * while playing; changing any other field rebuilds the stage (the picture
 * may stall for one frame), so drive sliders with live fields.
 *
 * Nodes act after the player's own orientation handling (container
 * rotation): a portrait video is upright before `transform` runs. At most
 * 64 nodes across all plugins.
 *
 * @example Brighten, sharpen and rotate the picture
 * The video plays a little brighter and more saturated, slightly sharper
 * and turned a quarter clockwise. The nodes run in array order; `set` sends
 * the whole chain as this plugin's desired value (last write wins), and the
 * player applies it to every source until it changes (the chain follows
 * the source by itself while it is set).
 * ```ts
 * const chain: VideoNode[] = [
 *   // Live fields: moving them later changes the picture in place.
 *   { kind: "eq", brightness: 0.05, contrast: 1.1, saturation: 1.2, gamma: 1 },
 *   // Not live: changing `amount` later rebuilds the stage (a one-frame stall).
 *   { kind: "sharpen", amount: 0.5, size: "small" },
 *   { kind: "transform", rotate: 90, flipH: false, flipV: false },
 * ];
 * plugin.video.filter("picture").set(chain);
 * ```
 *
 * @since 1.0.0
 * @category Processing
 */
export type VideoNode = 
/** FFmpeg `eq`; every field is live. */
{
    /** The node type. */
    kind: "eq";
    /** −1–1, 0 = unchanged. */
    brightness: number;
    /** 0–2, 1 = unchanged. */
    contrast: number;
    /** 0–2, 1 = unchanged. */
    saturation: number;
    /** 0.5–2, 1 = unchanged. */
    gamma: number;
}
/** Gaussian blur (FFmpeg `gblur`). */
 | {
    /** The node type. */
    kind: "gblur";
    /** 0–10 (live). */
    sigma: number;
    /** Always 3. */
    steps: 3;
}
/** Hue rotation; changes in place while playing. */
 | {
    /** The node type. */
    kind: "hue";
    /** −180–180 (live). */
    degrees: number;
}
/** unsharp: 3×3 (`small`) or 7×7 (`large`) luma matrix. */
 | {
    /** The node type. */
    kind: "sharpen";
    /** 0–2. */
    amount: number;
    /** Matrix size. */
    size: "small" | "large";
}
/** hqdn3d strengths. */
 | {
    /** The node type. */
    kind: "denoise";
    /** 0–20 (0 = off). */
    lumaSpatial: number;
    /** 0–20 (0 = off). */
    chromaSpatial: number;
    /** 0–20 (0 = off). */
    lumaTemporal: number;
    /** 0–20 (0 = off). */
    chromaTemporal: number;
}
/** FFmpeg `deblock`. */
 | {
    /** The node type. */
    kind: "deblock";
    /** Filter strength. */
    strength: "weak" | "strong";
}
/** FFmpeg `deband`. */
 | {
    /** The node type. */
    kind: "deband";
    /** 0.003–0.5, the same for all four planes. */
    threshold: number;
    /** Integer 1–64. */
    range: number;
}
/** Applied after the player's own source orientation. */
 | {
    /** The node type. */
    kind: "transform";
    /** Clockwise degrees; applied before the flips. */
    rotate: 0 | 90 | 180 | 270;
    /** Mirror horizontally. */
    flipH: boolean;
    /** Mirror vertically. */
    flipV: boolean;
}
/**
 * Deinterlacing. `interlaced` and `all` are FFmpeg BWDIF in the chain;
 * `field` doubles the frame rate.
 *
 * `when: "auto"` is the player's own deinterlacer instead: decided per frame
 * from the decoder's interlaced flag, with the decoder's hardware
 * deinterlacer where frames stay on the GPU; progressive frames pass
 * untouched. It needs no frames in memory, so a chain with only this node
 * keeps zero-copy hardware decoding (any other node still needs a copy back
 * to memory). It always outputs one frame per field, so it takes
 * `output: "field"` only (`"frame"` fails with `invalid`). It runs before
 * every plugin's chain wherever it is written, and is shared: on while any
 * plugin's chain has one. A chain with it has no other `deinterlace` node.
 * Snapshots and exports deinterlace the same frames with BWDIF at field rate.
 */
 | {
    /** The node type. */
    kind: "deinterlace";
    /**
     * `interlaced`: only frames flagged interlaced; `all`: every frame;
     * `auto`: the player's deinterlacer, on frames flagged interlaced.
     */
    when: "interlaced" | "all" | "auto";
    /**
     * One frame per frame, or one per field (double rate); `when: "auto"`
     * takes `field` only.
     */
    output: "frame" | "field";
}
/** Crop to / pad to a display aspect; x, y place it (−1 left/top … 1 right/bottom). */
 | {
    /** The node type. */
    kind: "reframe";
    /** Crop the picture, or pad it with black bars. */
    mode: "crop" | "pad";
    /** Target display aspect 0.25–4. */
    aspect: number;
    /** −1 (left) … 0 (centre) … 1 (right). */
    x: number;
    /** −1 (top) … 0 (centre) … 1 (bottom). */
    y: number;
}
/**
 * Blended frame-rate conversion (FFmpeg `framerate`), an integer 24–120.
 * At most one across all plugins; the player runs it after every chain.
 */
 | {
    /** The node type. */
    kind: "frameBlend";
    /** Output frame rate, integer 24–120. */
    fps: number;
}
/**
 * A convolution matrix of your own per plane (FFmpeg `convolution`): sharpen,
 * blur, emboss or detect edges with a 3×3, 5×5 or 7×7 kernel, or a row or
 * column kernel. `planes[i]` is plane `i` of the decoded picture: for the
 * usual YUV video 0 is luma (brightness), 1 and 2 are the colour planes, 3 is
 * alpha. A missing entry or `null` leaves that plane as it is; at least one
 * entry has a matrix.
 */
 | {
    /** The node type. */
    kind: "convolution";
    /** 1–4 entries, by plane; `null` passes the plane through. */
    planes: (ConvolutionPlane | null)[];
}
/**
 * Recolour the picture with a 3D look-up table (FFmpeg `lut3d`): a `.cube`
 * file the plugin ships in its package or wrote to its
 * {@link Plugin.dataDirectory}. The player reads the file when the chain is
 * set (and again for each new source, snapshot or export), so it must stay
 * there while the chain uses it. The table is applied to RGB; the player
 * converts the picture to RGB and back.
 */
 | {
    /** The node type. */
    kind: "lut3d";
    /**
     * Absolute path of a `.cube` file (`LUT_3D_SIZE` 2–65, at most
     * 16 MiB) inside the plugin's package directory or data directory
     * after following links; a path with `[` or `]` is refused. Outside
     * those: `invalid`; a file that does not exist: `missing`.
     *
     * The player checks only that header; the table itself is read by
     * FFmpeg's `.cube` reader (`parse_cube` in FFmpeg's
     * [`libavfilter/vf_lut3d.c`](https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/vf_lut3d.c)),
     * which takes a subset of the
     * Adobe/Resolve format:
     *
     * - Every line before `LUT_3D_SIZE N` is skipped (a `TITLE`, comments,
     *   and also a `DOMAIN_MIN` or `DOMAIN_MAX` there, which then has no
     *   effect).
     * - After it, optional `TITLE`, `DOMAIN_MIN r g b` and
     *   `DOMAIN_MAX r g b` lines (default `0 0 0` and `1 1 1`), blank
     *   lines and `#` comments, then N³ data lines of three numbers
     *   `r g b`, red changing fastest, then green, then blue.
     * - Any other keyword after `LUT_3D_SIZE`, such as `MIN` or `MAX`, is
     *   read as a data line, and FFmpeg refuses the file, as it does one
     *   with fewer than N³ data lines. The chain then fails and the
     *   player reports mpv's "error running command (-12)".
     *
     * ```text
     * TITLE "Identity"
     * LUT_3D_SIZE 2
     * DOMAIN_MIN 0 0 0
     * DOMAIN_MAX 1 1 1
     * 0 0 0
     * 1 0 0
     * 0 1 0
     * 1 1 0
     * 0 0 1
     * 1 0 1
     * 0 1 1
     * 1 1 1
     * ```
     */
    file: string;
    /** How colours between the table's points are computed; `tetrahedral` is FFmpeg's default. */
    interpolation: "nearest" | "trilinear" | "tetrahedral" | "pyramid" | "prism";
};
/**
 * One plane's kernel of a `convolution` {@link VideoNode}: each output
 * sample is `(sum of matrix × neighbourhood) × rdiv + bias`, clipped to the
 * sample range.
 *
 * @since 1.5.39
 * @category Processing
 */
export type ConvolutionPlane = {
    /**
     * `square`: `matrix` is a 3×3, 5×5 or 7×7 kernel, row by row (9, 25 or
     * 49 entries); `row` / `column`: a horizontal / vertical kernel with an
     * odd number of entries up to 49.
     */
    mode: "square" | "row" | "column";
    /** Integers −1024–1024 (FFmpeg reads whole numbers only). */
    matrix: number[];
    /**
     * Multiplier 0–256; 0 divides by the sum of the matrix (by 1 when it
     * sums to 0), which keeps the brightness of a blur.
     */
    rdiv: number;
    /** Added after the multiplication, 0–65535 in the plane's sample units (128 is mid grey for 8-bit video). */
    bias: number;
};
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
export declare const VIDEO_PROCESSING_LIMITS: Readonly<{
    /** Shaders across every plugin's `plugin.video.shaders()` handles (16). */
    shaders: 16;
    /** Bytes of one shader file (16 MiB). */
    shaderBytes: number;
    /** Bytes of a shader handle's id (128). */
    shaderIdBytes: 128;
    /** Bytes of one `lut3d` `.cube` file (16 MiB). */
    lutBytes: number;
    /** Largest `LUT_3D_SIZE` of a `lut3d` file, points per axis (65). */
    lutSize: 65;
    /** Bytes of a `lut3d` file path (4096). */
    lutPathBytes: 4096;
    /** Largest magnitude of a `convolution` matrix entry (1024). */
    matrixValue: 1024;
    /** Largest `convolution` `rdiv` (256). */
    rdiv: 256;
    /** Largest `convolution` `bias` (65535). */
    bias: 65535;
}>;
/**
 * One node of an {@link AudioFilter} chain, by `kind`. Values outside the
 * ranges fail with `invalid`; "live" fields change in place while playing.
 *
 * @example Lift the voices and play in mono
 * Speech around 1 kHz is 3 dB louder and both speakers play the same mixed
 * channel. `set` sends this plugin's whole audio chain (the nodes in order,
 * plus an optional `delayMs`); the player joins it with other plugins'
 * chains and applies it to every source.
 * ```ts
 * plugin.audio.filter("sound").set({
 *   nodes: [
 *     { kind: "equalizer", frequency: 1000, q: 1, gainDb: 3 }, // +3 dB at 1 kHz
 *     { kind: "channels", mode: "mono" }, // both channels get (L + R) / 2
 *   ],
 * });
 * ```
 *
 * @since 1.0.0
 * @category Processing
 */
export type AudioNode = 
/** FFmpeg `volume`. */
{
    /** The node type. */
    kind: "volume";
    /** −60–0 dB (live). */
    gainDb: number;
}
/** One peaking equalizer band (FFmpeg `equalizer`). */
 | {
    /** The node type. */
    kind: "equalizer";
    /** Centre frequency 20–20000 Hz. */
    frequency: number;
    /** Always 1. */
    q: 1;
    /** −6–6 dB (live). */
    gainDb: number;
}
/** Treble shelf (FFmpeg `treble`). */
 | {
    /** The node type. */
    kind: "treble";
    /** Always 3000 Hz. */
    frequency: 3000;
    /** Always 0.5. */
    slope: 0.5;
    /** 0–6 dB (live). */
    gainDb: number;
}
/** Dynamic loudness normalization (FFmpeg `dynaudnorm`); every field is live. */
 | {
    /** The node type. */
    kind: "dynaudnorm";
    /** On or off. */
    enabled: boolean;
    /** Maximum gain 1–4. */
    maxGain: number;
    /** Target peak 0.5–0.95. */
    peak: number;
}
/** Routing on a stereo mix (more channels are mixed down first). */
 | {
    /** The node type. */
    kind: "channels";
    /** `stereo` leaves it unchanged. */
    mode: "stereo" | "left" | "right" | "mono" | "swap";
};
/**
 * Whether `text` has a control character as Rust's `char::is_control`
 * sees one (C0, DEL and C1: U+0000–U+001F, U+007F–U+009F).
 */
export declare function hasControl(text: string): boolean;
/**
 * The form of a `lut3d` path (the player's `effects.rs lut_path`); where
 * the file is and what it holds the player checks when the request arrives
 * (the test host too, `TestHost.fileRoots`).
 */
export declare function lutPathProblem(file: unknown): string | null;
/**
 * Why `nodes` would be refused (`null` when valid): unknown kinds or fields,
 * values out of range, more than 64 nodes. Used by the test host.
 */
export declare function nodeProblem(stage: "video" | "audio", nodes: readonly {
    kind: string;
}[]): string | null;
export type ProcessingChange = {
    resourceId: string;
    revision: number;
    expectedGeneration: number;
} & ({
    stage: "video";
    nodes: VideoNode[];
} | {
    stage: "audio";
    nodes: AudioNode[];
    offsetMs?: number;
});
export interface ProcessingResource<N> {
    owner: number;
    revision: number;
    resource: {
        manifestId: string;
        resourceId: string;
        nodes: N[];
    };
    /** Present on audio resources. */
    offsetMs?: number;
}
export interface ProcessingState {
    configurationRevision: number;
    /** Sum of owned offsets, excluding device volume/mute. */
    audioOffsetMs: number;
    source: {
        generation: number;
        phase: "" | "empty" | "loading" | "ready" | "ended" | "error";
        /** Actual accepted FileLoaded identity; cycle here is load cycle, not a later seek cycle. */
        load: {
            entry: number;
            generation: number;
            cycle: number;
        } | null;
        stages: Partial<Record<"video" | "audio", {
            status: "waiting" | "applied" | "uncertain";
            error: string | null;
        }>>;
    };
    video: ProcessingResource<VideoNode>[];
    audio: ProcessingResource<AudioNode>[];
    /** Revocation may leave backend cleanup uncertain. Such a stage blocks updates. */
    backendUncertain: Partial<Record<"video" | "audio", string>>;
}
export interface ProcessingApplied {
    stage: "video" | "audio";
    revision: number;
    generation: number;
    status: "applied";
}
export type ProcessingToken = {
    stage: "video" | "audio";
    resourceId: string;
    revision: number;
};
export type ProcessingOperation = {
    status: "pending" | "unknown" | "unknownExpired";
} | {
    status: "completed";
    result: ProcessingApplied;
} | {
    status: "failed";
    error: {
        code: string;
        message: string;
    };
};
export declare function processingClient(transport: Transport): {
    state: () => Promise<ProcessingState>;
    /** Token is (stage, resourceId, revision). Fix it before sending. A dynamic
     * rejection consumes this token; never change its payload on retry. */
    replace: (change: ProcessingChange) => Promise<ProcessingApplied>;
    operation: (token: ProcessingToken) => Promise<ProcessingOperation>;
};
