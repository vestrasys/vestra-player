import type { Json } from "../../wire.ts";
import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** Implements `TestHost.setMpvProperty`. */
export declare function setMpvProperty(host: TestHost, name: string, value: Json): void;
/** The default handlers of this area's wire methods, by method name. */
export declare function mpvHandlers(host: TestHost, _inner: Internals): Record<string, Handler>;
