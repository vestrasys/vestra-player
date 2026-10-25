#!/usr/bin/env node
/**
 * Plugin-author documentation lives on the SDK's site
 * (https://vestrasys.github.io/vestra-player-sdk/), once. This check keeps
 * it from coming back into this repository's `doc/`:
 *
 * 1. `doc/plugins/` holds only `README.md` (a page of links to the site).
 * 2. No Markdown table in `doc/` re-copies one of the site's Reference
 *    tables. Heuristics, per table (consecutive lines starting with `|`):
 *    - capabilities: 8 or more rows whose first cell is one manifest
 *      capability name (the SDK's `CAPABILITIES`);
 *    - limits: 3 or more rows naming a limit constant of the SDK
 *      (`COMMAND_LIMITS.perPlugin`, `OSD_LIMITS`, `MESSAGE_LIMIT`, ...), or
 *      3 or more rows whose second cell starts with the value of one (8 or
 *      more; `64 KiB`, `1 MiB` count as their bytes);
 *    - platforms: a header with both `macOS` and `Windows` columns, 6 or
 *      more rows, and 3 or more of them naming an SDK interface or member
 *      (`Clipboard`, `MainWindow.setAlwaysOnTop`, `plugin.secrets`, ...) or
 *      the codes `unsupported` / `unavailable`.
 *    A table about the player's own implementation that trips a rule is
 *    rewritten to link the site, or reduced to what the site does not say.
 *
 *   node scripts/check-docs.mjs     (npm run docs:check; src/tests/sdk runs it)
 */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import * as runtime from "@vestra-player/sdk/runtime";

const repo = resolve(import.meta.dirname, "../..");
const doc = join(repo, "doc");
const problems = [];

const plugins = readdirSync(join(doc, "plugins"));
for (const name of plugins)
  if (name !== "README.md")
    problems.push(
      `doc/plugins/${name}: doc/plugins holds only README.md; author documentation belongs on the SDK site`,
    );

const capabilities = new Set(runtime.CAPABILITIES);
const limitObjects = Object.entries(runtime).filter(
  ([name, value]) =>
    name.endsWith("_LIMITS") && value && typeof value === "object",
);
const limitNames = new Set([
  ...limitObjects.map(([name]) => name),
  "MESSAGE_LIMIT",
]);
const limitValues = new Set(
  [
    ...limitObjects.flatMap(([, value]) => Object.values(value)),
    runtime.MESSAGE_LIMIT,
  ].filter((value) => typeof value === "number" && value >= 8),
);
const UNITS = { KiB: 1024, MiB: 1024 * 1024, KB: 1024, MB: 1024 * 1024 };
/** The number a cell starts with, in bytes when it has a unit. */
const leading = (cell) => {
  const match = /^(\d[\d_,]*)(?:\s*(KiB|MiB|KB|MB))?/.exec(cell);
  return match
    ? Number(match[1].replace(/[_,]/g, "")) * (UNITS[match[2]] ?? 1)
    : NaN;
};
const apis = new Set(
  Object.values(runtime.CAPABILITY_INFO).flatMap((info) => info.apis),
);
const interfaces = new Set([...apis].map((api) => api.split(".")[0]));

const cells = (line) =>
  line
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((cell) => cell.trim());
const bare = (cell) => cell.replace(/`/g, "").trim();
const namesApi = (cell) =>
  /\bplugin\.\w/.test(cell) ||
  [...cell.matchAll(/[A-Z]\w*(?:\.\w+)?/g)].some(
    ([word]) => apis.has(word) || interfaces.has(word),
  );

function check(file, text) {
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i].trimStart().startsWith("|")) continue;
    const start = i;
    const table = [];
    while (i < lines.length && lines[i].trimStart().startsWith("|"))
      table.push(cells(lines[i++]));
    const [header = [], , ...body] = table;
    const at = `${relative(repo, file)}:${start + 1}`;
    const count = (test) => body.filter(test).length;
    if (count((row) => capabilities.has(bare(row[0] ?? ""))) >= 8)
      problems.push(
        `${at}: a capability table; link the SDK site's Reference: Capabilities instead`,
      );
    const namesLimit = (row) =>
      row.some((cell) =>
        [...cell.matchAll(/\b[A-Z_]+_LIMITS?\b/g)].some(([w]) =>
          limitNames.has(w),
        ),
      );
    const valuedLimit = (row) => limitValues.has(leading(bare(row[1] ?? "")));
    if (count(namesLimit) >= 3 || count(valuedLimit) >= 3)
      problems.push(
        `${at}: a limits table; link the SDK site's Reference: Limits instead`,
      );
    if (
      header.some((cell) => /macOS/.test(cell)) &&
      header.some((cell) => /Windows/.test(cell)) &&
      body.length >= 6 &&
      count(
        (row) =>
          namesApi(row[0] ?? "") ||
          row.some((cell) => /`(unsupported|unavailable)`/.test(cell)),
      ) >= 3
    )
      problems.push(
        `${at}: a platform table of the plugin API; link the SDK site's Reference: Platforms instead`,
      );
  }
}

const walk = (directory) =>
  readdirSync(directory, { withFileTypes: true }).flatMap((item) => {
    const path = join(directory, item.name);
    return item.isDirectory()
      ? walk(path)
      : item.name.endsWith(".md")
        ? [path]
        : [];
  });
for (const file of walk(doc)) check(file, readFileSync(file, "utf8"));

if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("doc/: no copies of the SDK site's author documentation");
