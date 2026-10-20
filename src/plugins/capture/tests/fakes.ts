import { copyFile, stat } from "node:fs/promises";
import { HostError } from "../../../sdk/plugin/testing.ts";
import type { TestHost } from "../../../sdk/plugin/testing.ts";
import { sips } from "../sips.ts";

export type Fakes = {
  /** Paths given to `system.openPath`, newest last. */
  opened: string[];
  /** Image paths put on the clipboard, with whether the file existed at that moment. */
  copied: { path: string; existed: boolean }[];
  /** Conversions asked for, newest last. */
  converted: { source: string; target: string; format: string; quality: number }[];
  failOpen: boolean;
  failCopy: boolean;
  /** "error": sips fails; "silent": it exits 0 and writes nothing (like sips on a bad input). */
  failConvert: "error" | "silent" | null;
  /** While set, the clipboard write waits for it (to hold a copy in progress). */
  hold: Promise<void> | null;
};

const exists = (path: string) => stat(path).then(() => true, () => false);

/** `sips` replaced by a copy so outputs exist; reset per test with `fakes.reset()`. */
export function fakeSips() {
  const fakes: Fakes = { opened: [], copied: [], converted: [], failOpen: false, failCopy: false, failConvert: null, hold: null };
  sips.convert = async (source, target, format, quality) => {
    fakes.converted.push({ source, target, format, quality });
    if (fakes.failConvert === "error") throw new Error("sips failed");
    if (fakes.failConvert === "silent") return;
    await copyFile(source, target);
  };
  return fakes;
}

/** The test host's clipboard and Finder, observed (and failed on demand) through `fakes`. */
export function watchHost(host: TestHost, fakes: Fakes) {
  Object.assign(fakes, { opened: host.system.opened, copied: [], converted: [], failOpen: false, failCopy: false, failConvert: null, hold: null });
  host.handle("clipboard.writeImage", async (p) => {
    await fakes.hold;
    const path = String(p.path);
    fakes.copied.push({ path, existed: await exists(path) });
    if (fakes.failCopy) throw new HostError("failed", "the pasteboard refused the image");
    host.clipboard.images.push(path);
  });
  const open = host.local.open;
  host.local.open = async (path, reveal) => {
    if (fakes.failOpen) throw new HostError("failed", "open failed");
    await open(path, reveal);
  };
}
