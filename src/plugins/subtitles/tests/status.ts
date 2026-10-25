// What a window's status line and the OSD show (messages.ts `statusText`), as the test host translates it.
import { fileURLToPath } from "node:url";
import type { ListStyle, TemplateArgs, Text, TextArg, TextList } from "@vestra-player/sdk";
import { TestHost } from "@vestra-player/sdk/testing";
import { statusText } from "../messages.ts";
import type { NoteId, Status } from "../messages.ts";

/** `plugin.i18n.text` / `list`: plain text values, as the plugin makes them. */
export const words = {
  text: (source: string, args?: TemplateArgs): Text => ({ $vestraText: source, args }) as Text,
  list: (items: readonly TextArg[], style?: ListStyle): TextList => ({ $vestraList: items, style }),
};

/** The status line of a window (its props show text values translated): its status, else why styling is off. */
export const shownStatus = (props: Record<string, unknown>) => String(props.status ?? "") || String(props["style-reason"] ?? "");

/** A test host that shows the plugin's texts in Simplified Chinese (for pure functions). */
export function zhHost() {
  const host = new TestHost();
  host.locale = "zh-Hans";
  host.loadCatalog(fileURLToPath(new URL("..", import.meta.url)));
  return host;
}
/** A status (or a `Failure`'s) in Simplified Chinese, as a window shows it. */
export const zh = (status: { id: Status["id"]; args: TextArg[]; notes?: NoteId[] }, host = zhHost()) => {
  const shown = statusText(words, { notes: [], ...status });
  return typeof shown === "string" ? shown : host.translate(shown as Text);
};
