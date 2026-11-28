import type { Json, WireMotionState, WireRenderingState, WireUpscalingState } from "../../wire.ts";
import type { MotionEngine, UpscalingDecodingCause, UpscalingPlugin, UpscalingReason } from "../types.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/**
 * The player's `playback/plugin_files.rs`: a file the plugin names for its
 * video processing is an absolute path inside one of `roots` after
 * following links, a regular file of at most `max` bytes. Returns the real
 * path or throws the player's error.
 */
export declare function ownFile(path: string, roots: string[], what: string, max: number): string;
/** The `LUT_3D_SIZE` check of `plugin_files::lut` on the first 64 KiB. */
export declare function cubeProblem(file: string): string | null;
export declare function processingState(host: TestHost, _inner: Internals): {
    configurationRevision: number;
    audioOffsetMs: number;
    source: {
        generation: number;
        phase: "empty" | "loading" | "ready" | "ended" | "error";
        load: null;
        stages: {};
    };
    video: {
        offsetMs?: number | undefined;
        owner: number;
        revision: number;
        resource: {
            manifestId: string;
            resourceId: string;
            nodes: Json;
        };
    }[];
    audio: {
        offsetMs?: number | undefined;
        owner: number;
        revision: number;
        resource: {
            manifestId: string;
            resourceId: string;
            nodes: Json;
        };
    }[];
    backendUncertain: Partial<Record<"video" | "audio", string>>;
};
/** `video.upscaling.state` from {@link TestHost.upscaling}. */
export declare function upscalingState(host: TestHost, _inner: Internals, check?: boolean): WireUpscalingState;
/** `rendering.state` from {@link TestHost.rendering} and `videoRender.gpu`. */
export declare function renderingState(host: TestHost, _inner: Internals): WireRenderingState;
/** Implements `TestHost.failVideoRender`. */
export declare function failVideoRender(host: TestHost, _inner: Internals, code: "failed" | "unsupported" | "previous-crash", message: string): void;
/** Implements `TestHost.failVideoShaders`. */
export declare function failVideoShaders(host: TestHost, _inner: Internals, id: string, message: string, code?: "failed" | "previous-crash"): void;
/** Implements `TestHost.upscalingDecoding`. */
export declare function upscalingDecoding(host: TestHost, _inner: Internals, cause: UpscalingDecodingCause, plugins?: readonly UpscalingPlugin[]): UpscalingReason;
/** `video.motion.state` of an engine from its model (`amd` by default). */
export declare function motionState(host: TestHost, engine?: MotionEngine): WireMotionState;
/** The default handlers of this area's wire methods, by method name. */
export declare function processingHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
