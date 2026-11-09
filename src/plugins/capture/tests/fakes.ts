import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";
import { stat } from "node:fs/promises";
import { HostError } from "@vestra-player/sdk/testing";
import type { TestHost } from "@vestra-player/sdk/testing";

export type Fakes = {
  /** Paths given to `system.openPath`, newest last. */
  opened: string[];
  /** Image paths put on the clipboard, with whether the file existed at that moment. */
  copied: { path: string; existed: boolean }[];
  failOpen: boolean;
  failCopy: boolean;
  /** While set, the clipboard write waits for it (to hold a copy in progress). */
  hold: Promise<void> | null;
};

const exists = (path: string) => stat(path).then(() => true, () => false);

export const newFakes = (): Fakes => ({ opened: [], copied: [], failOpen: false, failCopy: false, hold: null });

/** The test host's clipboard and `system.openPath`, observed (and failed on demand) through `fakes`. */
export function watchHost(host: TestHost, fakes: Fakes) {
  Object.assign(fakes, newFakes());
  host.handle("clipboard.writeImage", async (p) => {
    await fakes.hold;
    const path = String(p.path);
    fakes.copied.push({ path, existed: await exists(path) });
    if (fakes.failCopy) throw new HostError("failed", "the pasteboard refused the image");
    host.clipboard.images.push(path);
  });
  host.handle("system.openPath", (p) => {
    if (fakes.failOpen) throw new HostError("failed", "open failed");
    fakes.opened.push(String(p.path));
  });
}

/** The JPEG/BMP still exports the plugin asked the player for, oldest first. */
export const stills = (host: TestHost) => host.exports.filter((job) => /(Jpeg|Bmp)$/.test(String(job.kind)));

/**
 * Wait until `read()` gives `expected` (deep, strict equality), then pass; fail
 * after about 6 s with the last value and `message`. The tab's values arrive in
 * the plugin's own updates, after the condition a test waited for: assert on
 * the value itself, never right after waiting on another one. A value already
 * there passes at once; a read that throws (a row not there yet) is retried.
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
  assert.deepStrictEqual(last, expected, message);
}
