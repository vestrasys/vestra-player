import type { ScopeImpl } from "./scope.ts";
import type { Clipboard, Host, Power, Secrets, Storage, System, Tools } from "./types.ts";
export declare const clipboard: (scope: ScopeImpl) => Clipboard;
export declare const host: (scope: ScopeImpl) => Host;
export declare const power: (scope: ScopeImpl) => Power;
export declare function tools(scope: ScopeImpl): Tools;
export declare const storage: (scope: ScopeImpl) => Storage;
export declare function system(scope: ScopeImpl): System;
export declare const secrets: (scope: ScopeImpl) => Secrets;
