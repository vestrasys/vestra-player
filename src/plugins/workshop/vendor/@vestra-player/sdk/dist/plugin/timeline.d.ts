import type { Runtime } from "./runtime.ts";
import type { ScopeImpl } from "./scope.ts";
import type { WireMark } from "../wire.ts";
import type { Timeline, TimelineMark } from "./types.ts";
/**
 * One per plugin: read before activation (with `playback.read`), then kept
 * current from `timeline.changed` (with `events`) once the plugin uses it.
 */
export declare class TimelineService {
    #private;
    marks: readonly TimelineMark[];
    layer: boolean;
    constructor(runtime: Runtime);
    init(): Promise<void>;
    start(): void;
    accept(marks: WireMark[]): void;
    read(): Promise<void>;
    watch(listener: (marks: readonly TimelineMark[]) => void): {
        dispose: () => Promise<undefined>;
    };
}
export declare const timelineOf: (runtime: Runtime) => TimelineService;
export declare function timeline(scope: ScopeImpl): Timeline;
