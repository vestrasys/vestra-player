// The command table Ask Vestra gives the model (PROPOSAL §3.1, §8;
// doc/architecture/palette-lead.md "Personal data"): the palette's rows from
// `plugin.commands.list()`, without the rows a plugin built from the user's
// data. The host cannot mark those (no `CommandOptions.personal` yet), so
// they are recognised here by owner and id, and by what a title looks like
// for plugins this plugin does not know.
import type { CommandInfo, CommandList } from "@vestra-player/sdk";

/**
 * Rows of built-in plugins whose titles are the user's data or the media's
 * metadata: Recently Played (file names), audio and subtitle tracks (track
 * titles, external subtitle file names), audio devices (device names),
 * chapters and bookmarks (titles from the file or the user).
 */
const PERSONAL: RegExp[] = [
  /^vestra\.history\/recent\.item\./,
  /^vestra\.audio\/audio\.track\./,
  /^vestra\.audio\/audio\.device\./,
  /^vestra\.marks\/chapter\./,
  /^vestra\.marks\/bookmark\./,
  /^vestra\.subtitles\/(track|second)-\d+$/,
];

/** Never offered: the palette itself (the host refuses it) and Ask Vestra's own commands. */
const NEVER = new Set(["host/palette"]);

/**
 * A title that reads like data rather than a command name: a file name
 * (an extension), a path or a URL. Applied to every row of a plugin that
 * is not built in, whose dynamic rows this plugin cannot know.
 */
const DATA_LIKE = /\\|:\/\/|^~?\/|\/[^/\s]+\/|\.[a-z][a-z0-9]{1,3}\b/i;

/** The rows the model may see and name. `self`: this plugin's manifest id. */
export function offered(list: CommandList, self: string): CommandInfo[] {
  return list.commands
    .filter((row) => !NEVER.has(row.id) && !row.id.startsWith(`${self}/`))
    .filter((row) => !PERSONAL.some((pattern) => pattern.test(row.id)))
    .filter((row) => row.builtin || !(DATA_LIKE.test(row.title) || row.path.some((segment) => DATA_LIKE.test(segment))))
    .sort((a, b) => a.labelSource.localeCompare(b.labelSource, "en") || a.id.localeCompare(b.id));
}

/**
 * One line per command, ordered by label so the table is the same between
 * requests while nothing changed (prompt caching):
 * `id | English label [| shown label] | place | keys | state`.
 */
export function commandTable(rows: CommandInfo[]): string {
  return rows
    .map((row) => {
      const state: string[] = [];
      if (!row.enabled) state.push(row.reason ? `disabled: ${row.reason}` : "disabled");
      if (row.busy) state.push("busy");
      if (row.checkable) state.push(`${row.radio ? "radio" : "toggle"} ${row.checked ? "on" : "off"}`);
      if (row.dangerous) state.push("asks the user first");
      if (!row.builtin) state.push(`third-party: ${row.plugin}`);
      const parts = [row.id, row.labelSource];
      if (row.label !== row.labelSource) parts.push(row.label);
      parts.push(row.place || "-", row.keys.join(" ") || "-", state.join(", ") || "-");
      return parts.join(" | ");
    })
    .join("\n");
}
