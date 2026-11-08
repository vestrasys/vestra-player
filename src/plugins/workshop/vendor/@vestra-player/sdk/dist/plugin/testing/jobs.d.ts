import type { Json } from "../../wire.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** `jobs.start`'s `encoding`, checked against the player's sets. */
export declare function validEncoding(value: Json | undefined): boolean;
/** Implements `TestHost.failStream`. */
export declare function failStream(host: TestHost, inner: Internals, producerId: number, message: string): void;
/** The default handlers of this area's wire methods, by method name. */
export declare function jobsHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
