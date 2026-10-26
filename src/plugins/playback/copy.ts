import type { Clipboard, PlayerState } from "@vestra-player/sdk";
import { fileName, folderOf, HOST_RULES, isLocalPath, type PathRules } from "./local-path.ts";
import { errorMessage, type Osd, type Reason } from "./texts.ts";

export type CopyKind = "name" | "folder" | "path";

/** I08: the text a copy command puts on the clipboard, or why it cannot. */
export function copyTarget(
  state: PlayerState,
  kind: CopyKind,
  rules: PathRules = HOST_RULES,
): { text: string } | { reason: Reason } {
  const { source } = state;
  if (!source || state.kind === "stream" || state.phase === "empty") return { reason: "No open file" };
  if (!isLocalPath(source, rules))
    return kind === "path" ? { text: source } : { reason: "A network source has no file name" };
  const text = kind === "name" ? fileName(source, rules) : kind === "folder" ? folderOf(source, rules) : source;
  return { text };
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
