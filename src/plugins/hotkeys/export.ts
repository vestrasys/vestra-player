import type { Line } from "./keys-page.ts";

const COLUMNS = ["命令", "快捷键", "范围", "来源", "状态"] as const;
const cells = (line: Line) => [line.command, line.keys, line.scope, line.source, line.status];

/** 导出列表… as CSV (UTF-8 with BOM so spreadsheet apps read the Chinese). */
export function exportCsv(lines: Line[]): string {
  const quote = (cell: string) => (/[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell);
  return `\u{feff}${[COLUMNS, ...lines.map(cells)].map((row) => row.map(quote).join(",")).join("\n")}\n`;
}

/** 导出列表… as plain text: the key list for reading or printing, one command a line. */
export function exportText(lines: Line[]): string {
  const rows = lines.map((line) => `${line.command}\t${line.keys || "—"}\t${line.scope}\t${line.source}\t${line.status}`);
  return `Vestra 快捷键列表\n\n${COLUMNS.join("\t")}\n${rows.join("\n")}\n`;
}
