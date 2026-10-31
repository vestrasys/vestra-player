import type { ScopeImpl } from "./scope.ts";
import type { VideoShaders } from "./types.ts";
/** The handle id when `shaders()` is called without one. */
export declare const DEFAULT_SHADERS_ID = "shaders";
/** Why `paths` would be refused here (`null` when it may be sent). */
export declare function shaderListProblem(paths: unknown): string | null;
/** Why a handle id would be refused (`null` when valid). */
export declare function shaderIdProblem(id: unknown): string | null;
export declare function videoShaders(scope: ScopeImpl, id?: string): VideoShaders;
