import type { ScopeImpl } from "./scope.ts";
/** `Scope.state`: a tracked copy of `initial`'s own fields. */
export declare function trackedState<T extends object>(scope: ScopeImpl, initial: T): T;
