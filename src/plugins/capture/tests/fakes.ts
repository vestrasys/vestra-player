import { stat } from "node:fs/promises";
import { HostError } from "@vestra-player/sdk/testing";
import type { TestHost } from "@vestra-player/sdk/testing";
import { REASON } from "../model.ts";
import type { Reason } from "../model.ts";

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

type Progress = { status: string; reason: string; raw: string; done: number; total: number };
type Status = { kind: string; reason: string; raw: string; count: number; name: string };
const STATES: Record<string, string> = {
  queued: "Queued",
  preparing: "Preparing",
  running: "Exporting",
  cancelling: "Cancelling",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
};
const STATUS: Record<string, string> = {
  saved: "{} frames saved",
  started: "Burst of {} frames started",
  "burst-failed": "Burst failed: {}",
  exported: "Exported {}",
  "clip-failed": "Export failed: {}",
  "no-destination": "No save location chosen.",
  "clip-started": "Export started. Follow it in Results.",
  restarted: "Restarted.",
};

/**
 * What the tab shows for the plugin's data, in the test host's language: a
 * mirror of `Words` in ui/parts.slint (the test host does not run Slint), with
 * the same English templates looked up in the plugin's catalog.
 */
export function words(host: TestHost) {
  const t = (text: string, args?: (string | number)[]) => host.translate(text, args);
  const reason = (code: string, raw: string) => (code ? t(REASON[code as Reason]) : raw);
  const state = (status: string) => (STATES[status] ? t(STATES[status]) : status);
  const progress = (p: Progress) =>
    !p.status
      ? ""
      : p.reason || p.raw
        ? t("Failed: {}", [reason(p.reason, p.raw)])
        : p.total > 1
          ? t("{0} · {1}/{2}", [state(p.status), p.done, p.total])
          : state(p.status);
  const status = (s: Status) => {
    if (s.kind === "error") return reason(s.reason, s.raw);
    const template = STATUS[s.kind];
    if (!template) return "";
    const why = reason(s.reason, s.raw);
    return t(template, s.kind === "saved" || s.kind === "started" ? [s.count] : s.kind === "exported" ? [s.name] : s.kind.endsWith("failed") ? [why] : undefined);
  };
  return { reason, progress, status, STATES, STATUS };
}
