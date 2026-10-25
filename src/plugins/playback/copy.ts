import { basename, dirname } from "node:path";
import type { Clipboard, PlayerState } from "@vestra-player/sdk";
import { errorMessage, type Osd, type Reason } from "./texts.ts";

export type CopyKind = "name" | "folder" | "path";

/** I08: the text a copy command puts on the clipboard, or why it cannot. */
export function copyTarget(state: PlayerState, kind: CopyKind): { text: string } | { reason: Reason } {
  const { source } = state;
  if (!source || state.kind === "stream" || state.phase === "empty") return { reason: "No open file" };
  if (!source.startsWith("/")) return kind === "path" ? { text: source } : { reason: "A network source has no file name" };
  return { text: kind === "name" ? basename(source) : kind === "folder" ? dirname(source) : source };
}

/** Over 60 characters: the first 20 and the last 37 around an ellipsis. */
export function shorten(text: string) {
  const chars = Array.from(text);
  return chars.length > 60 ? `${chars.slice(0, 20).join("")}…${chars.slice(-37).join("")}` : text;
}

export async function copyToClipboard(
  clipboard: Clipboard,
  state: PlayerState,
  kind: CopyKind,
  osd: Osd,
) {
  const target = copyTarget(state, kind);
  if ("reason" in target) return;
  try {
    await clipboard.writeText(target.text);
  } catch (error) {
    osd("Copy failed: {}", [errorMessage(error)]);
    return;
  }
  osd("Copied: {}", [shorten(target.text)]);
}
