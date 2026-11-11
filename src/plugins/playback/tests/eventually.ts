import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";

/**
 * How long a wait may take before it fails: a ceiling, not a delay. A passing
 * wait returns at the first poll that sees its value (every 5 ms), so the
 * ceiling only matters on a loaded machine (the suites run in parallel, CI
 * runners are shared), where a 2 s ceiling once failed a debounced save.
 */
export const WAIT_MS = 10_000;
const POLL_MS = 5;

/**
 * Wait until `read()` deep-equals `expected` (up to {@link WAIT_MS}), then
 * return; otherwise fail with the last value read. Wait on the value you
 * assert, never on another property followed by an assertion right after it.
 */
export async function eventually(read: () => unknown, expected: unknown, message?: string) {
  const deadline = Date.now() + WAIT_MS;
  let last: unknown;
  let failure: unknown;
  for (;;) {
    try {
      last = read();
      failure = undefined;
      if (isDeepStrictEqual(last, expected)) return;
    } catch (error) {
      failure = error;
    }
    if (Date.now() > deadline) break;
    await new Promise((done) => setTimeout(done, POLL_MS));
  }
  if (failure) throw failure;
  if (message === undefined) assert.deepStrictEqual(last, expected);
  else assert.deepStrictEqual(last, expected, message);
}

/**
 * Poll `predicate` until it holds (up to {@link WAIT_MS}). For conditions that
 * are not one value to compare (a held request has started, a file parses);
 * where the test then asserts a value, prefer {@link eventually} on it.
 */
export async function until(predicate: () => boolean | Promise<boolean>, ms = WAIT_MS) {
  const deadline = Date.now() + ms;
  while (!(await predicate())) {
    if (Date.now() > deadline) throw new Error(`plugin did not settle within ${ms} ms`);
    await new Promise((done) => setTimeout(done, POLL_MS));
  }
}
