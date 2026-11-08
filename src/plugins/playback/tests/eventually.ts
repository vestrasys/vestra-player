import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";

/**
 * Wait until `read()` deep-equals `expected` (up to 6 s), then return; otherwise
 * fail with the last value read. Wait on the value you assert, never on another
 * property followed by an assertion right after it.
 */
export async function eventually(read: () => unknown, expected: unknown, message?: string) {
  let last: unknown;
  let failure: unknown;
  for (let i = 0; i < 1200; i++) {
    try {
      last = read();
      failure = undefined;
      if (isDeepStrictEqual(last, expected)) return;
    } catch (error) {
      failure = error;
    }
    await new Promise((done) => setTimeout(done, 5));
  }
  if (failure) throw failure;
  if (message === undefined) assert.deepStrictEqual(last, expected);
  else assert.deepStrictEqual(last, expected, message);
}
