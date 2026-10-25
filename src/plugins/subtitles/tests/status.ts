// What a window's status line shows (ui/messages.slint), as the test host translates it.
import { fileURLToPath } from "node:url";
import type { TextArg } from "@vestra-player/sdk";
import { TestHost } from "@vestra-player/sdk/testing";
import { STATUS } from "../messages.ts";
import type { StatusId } from "../messages.ts";

type Slint = { id: string; a: string; b: string; c: string; n1: string; n2: string };
export function statusText(host: TestHost, value: unknown): string {
  const s = value as Slint;
  if (!s || s.id === "") return "";
  const head = s.id === "raw" ? s.a : host.translate(STATUS[s.id as StatusId], [s.a, s.b, s.c]);
  return [s.n1, s.n2]
    .filter(Boolean)
    .reduce((text, note) => host.translate("{0}. {1}", [text, host.translate(STATUS[note as StatusId])]), head);
}
/** The status line of a window: its status, else why styling is off. */
export const shownStatus = (host: TestHost, props: Record<string, unknown>) =>
  statusText(host, props.status) || statusText(host, props["style-reason"]);

/** A test host that shows the plugin's texts in Simplified Chinese (for pure functions). */
export function zhHost() {
  const host = new TestHost();
  host.locale = "zh-Hans";
  host.loadCatalog(fileURLToPath(new URL("..", import.meta.url)));
  return host;
}
/** A status (or a `Failure`'s) in Simplified Chinese, as a window shows it. */
export const zh = (status: { id: string; args: TextArg[]; notes?: string[] }) => {
  const host = zhHost();
  const [a = "", b = "", c = ""] = status.args.map((arg) => (typeof arg === "object" ? host.translate(arg) : String(arg)));
  const [n1 = "", n2 = ""] = status.notes ?? [];
  return statusText(host, { id: status.id, a, b, c, n1, n2 });
};
