import type { PluginError } from "./types.ts";
export declare const sleep: (ms: number) => Promise<void>;
export declare function toError(error: unknown): PluginError;
export declare const failure: (code: string, message?: string) => PluginError;
/** Host replies are definite; anything else (lost reply, client timeout) is not. */
export declare const uncertain: (error: unknown) => boolean;
/** Mark a promise as observed so an unused `ready` never crashes the runner. */
export declare function quiet<T>(promise: Promise<T>): Promise<T>;
export declare function abortError(): PluginError;
