import type { Json } from "../../wire.ts";
import type { SourceState } from "../../source.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
export declare function initialSource(): SourceState;
export declare const readyCapabilities: {
    pause: boolean;
    seek: boolean;
    rate: boolean;
    restart: boolean;
    close: boolean;
    snapshotSource: boolean;
};
export declare function sourceIntent(host: TestHost, inner: Internals, intent: Record<string, Json>): {
    ticket: Json;
};
/** The file resolver of this plugin that claims `source`, if any. */
export declare function fileResolver(host: TestHost, _inner: Internals, source: string): string | null;
/** Retire the producer that is the player's source, unless it is `keep`. */
export declare function retireSelected(host: TestHost, inner: Internals, keep: number | null): void;
/** Whether producer `id` is the player's source. */
export declare function isCurrentProducer(host: TestHost, inner: Internals, id: number): boolean;
/** Start resolving `source` for the open just made; its request id, or null. */
export declare function startResolution(host: TestHost, inner: Internals, source: string, paused: boolean, operation?: number): number | null;
/** Settle a resolution's open; `error` fails it. */
export declare function settleResolution(host: TestHost, inner: Internals, requestId: number, error?: string): void;
/** Resolutions whose open is no longer the source (or whose resolver left). */
export declare function cancelResolutions(host: TestHost, inner: Internals): void;
/** `sourceResolvers.register`: the player's rules for both forms. */
export declare function registerResolver(host: TestHost, inner: Internals, p: Record<string, Json>): void;
/** `sourceResolvers.respond` for a simulated file resolution. */
export declare function respondResolution(host: TestHost, inner: Internals, p: Record<string, Json>): {
    stamp: {
        generation: number;
        intentVersion: number;
    };
} | null;
/** Audio track calls need ready media and the source the caller saw. */
export declare function audioCondition(host: TestHost, _inner: Internals, expected: Json | undefined): void;
/** `stale` unless `expected` is the current subtitle stamp. */
export declare function subtitleExpected(host: TestHost, _inner: Internals, expected: Json): void;
export declare function loadedSubtitles(host: TestHost, inner: Internals): Map<string, number>;
/**
 * The player's `SUBTITLE_LIMITS` for a new resource (`busy`, as the
 * player reports them). External tracks in `subtitles.tracks` that this
 * plugin did not load count as other plugins'. `totalBytes` is not
 * modelled: one plugin's 4 files of at most 2 MiB stay below it, and the
 * bytes of other plugins' tracks are unknown.
 */
export declare function subtitleRoom(host: TestHost, inner: Internals, resourceId: string): void;
export declare function subtitleChange(host: TestHost, inner: Internals, expected: Json): void;
/** Implements `TestHost.setSource`. */
export declare function setSource(host: TestHost, _inner: Internals, patch: Partial<SourceState>): void;
/** The default handlers of this area's wire methods, by method name. */
export declare function mediaHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
