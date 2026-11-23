import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { Subtitles, SubtitlesState } from "./types.ts";
/**
 * The player's limits on subtitle files a plugin loads with
 * {@link Subtitles.load}, which `TestHost` enforces with the player's codes.
 * The track counts are per source: opening another file starts from zero.
 * A file over `fileBytes` fails with `invalid`; every other limit fails with
 * `busy`, and nothing is loaded.
 *
 * @remarks
 * Each `load` uses a new resource on the source, and removing the track
 * frees its place among the live tracks (`tracksPerPlugin`, `tracks`,
 * `totalBytes`) but not among the loads (`loadsPerPlugin`, `loads`). So one
 * plugin loads at most 16 files into one source, with at most 4 of them
 * present at once.
 *
 * @example Replace the plugin's oldest subtitle when it has four
 * The plugin keeps the files it loaded, newest last, and removes the oldest
 * before a fifth would be refused with `busy`. A load the player still
 * refuses (the source holds 16 tracks of every plugin, or this plugin
 * already loaded 16 files into it) is reported on the OSD.
 * ```ts
 * import type { Plugin, PluginError, SubtitleTrack } from "@vestra-player/sdk";
 * declare const plugin: Plugin;
 * declare const bytes: Uint8Array;
 * // ---cut---
 * import { SUBTITLE_LIMITS } from "@vestra-player/sdk/testing";
 *
 * const loaded: SubtitleTrack[] = []; // this source's, newest last
 * if (loaded.length >= SUBTITLE_LIMITS.tracksPerPlugin)
 *   await plugin.subtitles.remove(loaded.shift()!);
 * try {
 *   loaded.push(await plugin.subtitles.load(bytes));
 * } catch (error) {
 *   if ((error as PluginError).code !== "busy") throw error;
 *   await plugin.osd.show("No room for another subtitle on this file.");
 * }
 * ```
 *
 * @since 1.5.34
 *
 * @category Testing
 */
export declare const SUBTITLE_LIMITS: Readonly<{
    /** UTF-8 bytes of one file; a larger one fails with `invalid` (2 MiB). */
    readonly fileBytes: number;
    /** Tracks one plugin has loaded into the current source at the same time (4). */
    readonly tracksPerPlugin: 4;
    /** Loaded tracks of every plugin in the current source at the same time (16). */
    readonly tracks: 16;
    /** Bytes of the loaded tracks of every plugin in the current source together (16 MiB). */
    readonly totalBytes: number;
    /** Loads by one plugin into the current source, removed tracks included (16). */
    readonly loadsPerPlugin: 16;
    /** Loads by every plugin into the current source, removed tracks included (128). */
    readonly loads: 128;
    /** Loads of one plugin in progress at the same time (2). */
    readonly uploadsPerPlugin: 2;
    /** Loads of every plugin in progress at the same time (8). */
    readonly uploads: 8;
}>;
/** `subtitles.changed`'s data as the listener gets it (`ScopeImpl.fact`). */
export declare function subtitlesChanged(runtime: Runtime, data: unknown): {
    state: SubtitlesState;
};
export declare function subtitles(scope: ScopeImpl): Subtitles;
