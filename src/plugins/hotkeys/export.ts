import { STATE } from "./keys.ts";
import type { Line } from "./keys-page.ts";

// i18n: the exported file is written here, not shown by the player, so its
// words cannot be translated yet (round 2 request R2-hotkeys-1 in
// doc/process/rounds/2026-10-i18n.md); they stay English. Command paths
// are the player's, in the interface language.
const COLUMNS = ["Command", "Shortcut", "Scope", "Source", "Status"] as const;
const STATUS: Record<number, string> = {
  [STATE.default]: "Default",
  [STATE.changed]: "Changed",
  [STATE.conflict]: "Conflict",
  [STATE.ignored]: "Ignored",
  [STATE.orphan]: "Not loaded",
};
const keysOf = (line: Line) => (line.state === STATE.ignored && line.keys ? `(${line.keys})` : line.keys);
const cells = (line: Line) => [
  line.command,
  keysOf(line),
  line.scope === "panel" ? "In panel" : line.scope === "global" ? "Global" : "—",
  line.source === "host" ? "Player" : line.source,
  STATUS[line.state] ?? "",
];

/** Export List… as CSV (UTF-8 with BOM so spreadsheet apps read non-Latin command names). */
export function exportCsv(lines: Line[]): string {
  const quote = (cell: string) => (/[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell);
  return `\u{feff}${[COLUMNS, ...lines.map(cells)].map((row) => row.map(quote).join(",")).join("\n")}\n`;
}

/** Export List… as plain text: the key list for reading or printing, one command a line. */
export function exportText(lines: Line[]): string {
  const rows = lines.map((line) => {
    const [command, keys, scope, source, status] = cells(line);
    return `${command}\t${keys || "—"}\t${scope}\t${source}\t${status}`;
  });
  return `Vestra Player shortcuts\n\n${COLUMNS.join("\t")}\n${rows.join("\n")}\n`;
}
