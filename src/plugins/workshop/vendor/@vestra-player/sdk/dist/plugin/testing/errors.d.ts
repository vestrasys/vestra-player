/**
 * Host errors the plugin logged (SDK 1.11.2): a refusal or failure with a
 * code (`HostError`, e.g. `stale`, `invalid`, `disabled`) that nothing in
 * the plugin handled, so it reached the plugin's log, as the player's log
 * would show it. Under `TestHost.strictErrors` (the default) such an error
 * fails the test at the next `settle()` (also `command(…).run()`) or at
 * `stop()`, unless the test expected it with `expectError(code)`.
 *
 * Part of `TestHost` (`./index.ts`). Internal to the SDK; not exported from
 * `@vestra-player/sdk/testing`.
 */
import { HostError } from "../../wire.ts";
import type { TestHost } from "./index.ts";
/** One host's book of logged errors and expectations. */
export declare class ErrorBook {
    /** Every host error the plugin logged, expected or not, in order. */
    readonly logged: HostError[];
    /** Logged errors no expectation covered, not yet thrown. */
    readonly unexpected: HostError[];
    /** Codes the test expects, one entry per expected error. */
    readonly expected: string[];
    /** Above 0 while `startPlugin` itself settles (activation, `stop`, `park`). */
    suspended: number;
}
/** The book of each host (`TestHost` makes it in its constructor). */
export declare const errorBooks: WeakMap<TestHost, ErrorBook>;
/**
 * The runtime's `logged` hook (`Connection.logged`): note an error that
 * reached the plugin's log. Errors without a code of the host's kind (a
 * `TypeError` in the plugin's code) are not counted.
 */
export declare function noteLogged(host: TestHost, error: unknown): void;
/**
 * Throw the unexpected host errors logged since the last check, once
 * (they are cleared), when the host is strict.
 */
export declare function checkErrors(host: TestHost): void;
/** At `stop()`: the unexpected errors, then the expected ones that never came. */
export declare function checkErrorsAtStop(host: TestHost): void;
