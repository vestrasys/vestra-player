import type { ScopeImpl } from "./scope.ts";
import type { AudioFilter, VideoFilter } from "./types.ts";
export declare function videoFilters(scope: ScopeImpl): {
    filter(id: string): VideoFilter;
};
export declare function audioFilters(scope: ScopeImpl): {
    filter(id: string): AudioFilter;
};
