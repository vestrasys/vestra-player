import type { ScopeImpl } from "./scope.ts";
import type { Bindings } from "./types.ts";
/** Throws `invalid` unless `text` is a canonical binding; returns it unchanged. */
export declare function canonical(text: string): string;
export declare function checkBindings(bindings: readonly string[]): string[];
/** `plugin.bindings` (`bindings.manage`): the table and the user overlay. */
export declare const bindingsApi: (scope: ScopeImpl) => Bindings;
