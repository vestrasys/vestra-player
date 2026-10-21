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
