import type { I18n } from "@vestra-player/sdk";
import { STATE } from "./keys.ts";
import type { Line } from "./keys-page.ts";

// The exported file leaves the player: its words are resolved here in the
// interface language. Command paths are the player's, already translated.
const COLUMNS = ["Command", "Shortcut", "Scope", "Source", "Status"] as const;
const STATUS = {
  [STATE.default]: "Default",
  [STATE.changed]: "Changed",
  [STATE.conflict]: "Conflict",
  [STATE.ignored]: "Ignored",
  [STATE.orphan]: "Not loaded",
} as const;
const cells = ({ text, resolve }: I18n, line: Line): string[] => {
  const keys = line.state === STATE.ignored && line.keys ? resolve(text("({})", [line.keys])) : line.keys;
  const status = Object.hasOwn(STATUS, line.state) ? resolve(text(STATUS[line.state as keyof typeof STATUS])) : "";
  return [
    line.command,
    keys,
    line.scope === "panel" ? resolve(text("In panel")) : line.scope === "global" ? resolve(text("Global")) : "—",
    line.source === "host" ? resolve(text("Player")) : line.source,
    status,
  ];
};

/** Export List… as CSV (UTF-8 with BOM so spreadsheet apps read non-Latin command names). */
export function exportCsv(i18n: I18n, lines: Line[]): string {
  const quote = (cell: string) => (/[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell);
  const header = COLUMNS.map((column) => i18n.resolve(i18n.text(column)));
  return `\u{feff}${[header, ...lines.map((line) => cells(i18n, line))].map((row) => row.map(quote).join(",")).join("\n")}\n`;
}

/** Export List… as plain text: the key list for reading or printing, one command a line. */
export function exportText(i18n: I18n, lines: Line[]): string {
  const rows = lines.map((line) => {
    const [command, keys, scope, source, status] = cells(i18n, line);
    return `${command}\t${keys || "—"}\t${scope}\t${source}\t${status}`;
  });
  const header = COLUMNS.map((column) => i18n.resolve(i18n.text(column)));
  return `${i18n.resolve(i18n.text("Vestra Player shortcuts"))}\n\n${header.join("\t")}\n${rows.join("\n")}\n`;
}
