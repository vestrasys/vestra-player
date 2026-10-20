import { basename, dirname } from "node:path";
import type { Clipboard, PlayerState } from "../../sdk/plugin/index.ts";

export type CopyKind = "name" | "folder" | "path";

/** I08: the text a copy command puts on the clipboard, or why it cannot. */
export function copyTarget(state: PlayerState, kind: CopyKind): { text: string } | { reason: string } {
  const { source } = state;
  if (!source || state.kind === "stream" || state.phase === "empty") return { reason: "没有打开的文件" };
  if (!source.startsWith("/")) return kind === "path" ? { text: source } : { reason: "网络来源没有文件名" };
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
  osd: (text: string) => void,
) {
  const target = copyTarget(state, kind);
  if ("reason" in target) return;
  try {
    await clipboard.writeText(target.text);
  } catch (error) {
    osd(`复制失败：${error instanceof Error ? error.message : String(error)}`);
    return;
  }
  osd(`已复制: ${shorten(target.text)}`);
}
