// What a window's status line shows (ui/messages.slint), as the test host translates it.
import type { TestHost } from "@vestra-player/sdk/testing";
import { STATUS } from "../messages.ts";
import type { StatusId } from "../messages.ts";

type Line = { id: string; a: string; b: string };
type Slint = { kind: string; service: string; p0: Line; p1: Line; p2: Line; p3: Line; p4: Line };

/** ui/messages.slint `Messages.location`, the same English texts. */
export const LOCATIONS: Record<string, string> = {
  macos: "macOS Keychain ({})",
  windows: "Windows Credential Manager ({})",
  system: "System secret store ({})",
  file: "{} (readable only by you)",
  memory: "This run only",
};
export const location = (host: TestHost, value: unknown) => {
  const l = value as { kind: string; value: string };
  return host.translate(LOCATIONS[l.kind] ?? LOCATIONS.memory!, [l.value]);
};
export const serviceName = (host: TestHost, id: string) => (id === "assrt" ? host.translate("Assrt") : "OpenSubtitles");

export function lineText(host: TestHost, value: unknown): string {
  const l = value as Line;
  if (!l || l.id === "") return "";
  if (l.id === "raw") return l.a;
  if (l.id === "saved") return host.translate(STATUS.saved, [location(host, { kind: l.a, value: l.b })]);
  const text = STATUS[l.id as StatusId];
  return host.translate(text, [l.a, l.b].slice(0, text.split("{}").length - 1));
}

/** The text of a window's `status` property. */
export function statusText(host: TestHost, value: unknown): string {
  const s = value as Slint;
  if (s.kind === "service") return host.translate("{0}: {1}", [serviceName(host, s.service), lineText(host, s.p0)]);
  const join = (first: string, next: string) =>
    next === "" ? first : first === "" ? next : host.translate(s.kind === "list" ? "{0}; {1}" : "{0}. {1}", [first, next]);
  return [s.p0, s.p1, s.p2, s.p3, s.p4].map((l) => lineText(host, l)).reduce(join);
}
