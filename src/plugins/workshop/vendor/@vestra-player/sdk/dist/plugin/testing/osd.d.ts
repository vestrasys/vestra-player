import type { TestHost } from "./index.ts";
import type { Handler, Internals } from "./internals.ts";
/** The default handlers of this area's wire methods, by method name. */
export declare function osdHandlers(host: TestHost, inner: Internals): Record<string, Handler>;
