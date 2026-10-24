// Builds the plugin toolchain the AI's tools run (vendor/) and the reference
// the agent reads (knowledge/), from the SDK the player is built with
// (src/node_modules/@vestra-player/sdk, locked by src/package-lock.json).
//
//   npm run vendor:sdk           rebuild and write into the plugin
//   node tests/vendor-sdk.mjs    rebuild into a temporary directory and list differences
//
// vendor/ is laid out like a node_modules folder without being one (git
// ignores node_modules/): the CLI finds TypeScript through NODE_PATH.
//
//   vendor/@vestra-player/sdk/   package.json, LICENSE, README.md, bin/ (vestra-plugin CLI),
//                                dist/ (manifest rules, TestHost, .d.ts), slint/ (widget library)
//   vendor/typescript/           the SDK's TypeScript: tsc and the compiler API, lib .d.ts
//   vendor/@types/node/          the SDK's @types/node, and its undici-types
//   vendor/undici-types/
//
// Maintainer's alternative to R-h (round 2026-10-vestra-ai): the player's
// runtime does not ship the CLI, so this plugin brings it along.
//
// knowledge/ (read-only for the agent): llms.txt, an index of the API made
// from the SDK's .d.ts and README, and llms-full.txt (index, README and the
// eight examples, tests/upstream/sdk-examples.md). When the SDK ships its own
// llms.txt / llms-full.txt (planned for 1.5.6) they are copied with the
// package and knowledge/ is not written.
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(path.join(plugin, "package.json"));

/** The installed package roots: the SDK and what it resolves. */
export function sources() {
  const sdk = path.dirname(require.resolve("@vestra-player/sdk/package.json"));
  const from = createRequire(path.join(sdk, "package.json"));
  const typescript = path.dirname(from.resolve("typescript/package.json"));
  const node = path.dirname(from.resolve("@types/node/package.json"));
  const undici = path.dirname(createRequire(path.join(node, "package.json")).resolve("undici-types/package.json"));
  return { sdk, typescript, node, undici };
}

const SDK_FILES = ["package.json", "LICENSE", "README.md", "bin", "dist", "slint", "llms.txt", "llms-full.txt"];
/** tsc (lib/tsc.js loads lib/_tsc.js), the compiler API the CLI transpiles with, and the lib declarations. */
const TS_FILE = (name) => /^(tsc|_tsc|typescript)\.js$/.test(name) || /^lib\..*\.d\.ts$/.test(name);

async function copyTree(from, to, keep = () => true) {
  await fs.mkdir(to, { recursive: true });
  for (const item of await fs.readdir(from, { withFileTypes: true })) {
    if (item.name === "node_modules" || !keep(item.name)) continue;
    const source = path.join(from, item.name);
    const target = path.join(to, item.name);
    if (item.isDirectory()) await copyTree(source, target);
    else if (item.isFile()) await fs.copyFile(source, target);
  }
}

/** First sentence of a TSDoc comment, one line. */
function summary(ts, node) {
  const doc = node.jsDoc?.[0];
  const text = doc ? (ts.getTextOfJSDocComment(doc.comment) ?? "") : "";
  const flat = text.replace(/\{@link\s+([^}|\s]+)[^}]*\}/g, "$1").replace(/\s+/g, " ").trim();
  const end = flat.search(/\.(\s|$)/);
  const first = end >= 0 ? flat.slice(0, end + 1) : flat;
  return first.length > 160 ? `${first.slice(0, 159)}…` : first;
}

/** The index part of llms.txt: modules of Scope, then every declaration with its line. */
async function apiIndex(sdk) {
  const ts = (await import("typescript")).default;
  const lines = [];
  const files = ["dist/plugin/types.d.ts", "dist/plugin/testing.d.ts"];
  const declarations = [];
  let scope = [];
  for (const rel of files) {
    const text = await fs.readFile(path.join(sdk, rel), "utf8");
    const file = ts.createSourceFile(rel, text, ts.ScriptTarget.Latest, true);
    const line = (node) => file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1;
    for (const statement of file.statements) {
      const name = statement.name?.text ?? statement.declarationList?.declarations?.[0]?.name?.text;
      if (!name) continue;
      const kind = ts.isInterfaceDeclaration(statement)
        ? "interface"
        : ts.isTypeAliasDeclaration(statement)
          ? "type"
          : ts.isClassDeclaration(statement)
            ? "class"
            : ts.isFunctionDeclaration(statement)
              ? "function"
              : "const";
      declarations.push(`- ${kind} ${name} — ${rel}:${line(statement)}${summary(ts, statement) ? ` — ${summary(ts, statement)}` : ""}`);
      if (ts.isInterfaceDeclaration(statement) && (name === "Scope" || name === "Plugin"))
        for (const member of statement.members) {
          const memberName = member.name?.getText(file);
          if (memberName) scope.push(`- plugin.${memberName} — ${rel}:${line(member)}${summary(ts, member) ? ` — ${summary(ts, member)}` : ""}`);
        }
    }
  }
  const { CAPABILITIES, API_VERSION } = await import(pathToFileURL(path.join(sdk, "dist/capabilities.js")).href); // boundary-reviewed: the installed SDK's capability table, at vendoring time
  lines.push(
    "## The plugin object: modules and the capability each needs",
    "",
    "`activate(plugin)` receives a Plugin (a Scope). Each module below documents its manifest capability in parentheses.",
    "",
    ...scope,
    "",
    `## Capabilities (manifest \`capabilities\`, apiVersion \`${API_VERSION}\`)`,
    "",
    CAPABILITIES.map((c) => `\`${c}\``).join(", "),
    "",
    "## Every declaration (read the line range with `read` before using it)",
    "",
    ...declarations,
  );
  return lines.join("\n");
}

async function knowledge(sdk, out) {
  const pkg = JSON.parse(await fs.readFile(path.join(sdk, "package.json"), "utf8"));
  const head = [
    `# Vestra Player plugin SDK ${pkg.version} (@vestra-player/sdk) — reference for the AI`,
    "",
    "Made by Vestra Player AI from the SDK's README and .d.ts files (tests/vendor-sdk.mjs) until the SDK ships its own llms.txt.",
    "{SDK} is the read-only SDK folder named in the system prompt; paths below are relative to it.",
    "",
    "## Files",
    "",
    "- {SDK}/README.md — overview, quick start, command line",
    "- {SDK}/dist/plugin/types.d.ts — the whole plugin API with TSDoc and examples (search it with grep, read line ranges)",
    "- {SDK}/dist/plugin/testing.d.ts — the test host for tests/*.test.ts: startPlugin, TestHost",
    "- {SDK}/slint/vestra/*.slint, {SDK}/slint/palette.slint — the player's Slint library (`@vestra/theme.slint`, `@vestra/widgets.slint`, …)",
    "- llms-full.txt (next to this file) — this index, the README and the source of the eight examples",
    "",
    "## Examples (full source in llms-full.txt)",
    "",
    "- hello — one menu command and an on-screen message",
    "- blur — a tool window with one slider that blurs the video",
    "- signal-generator — generated video and audio frames",
    "- bookmark — a tool window bound to the playback state",
    "- panel-menu — a sidebar panel with its own right-click menu",
    "- menus — radio items, separator groups, a changing title",
    "- theme-widgets — a tool window drawn with the player's theme and widgets",
    "- markdown — a window that shows a Markdown document",
    "",
  ].join("\n");
  const index = await apiIndex(sdk);
  const llms = `${head}\n${index}\n`;
  const readme = (await fs.readFile(path.join(sdk, "README.md"), "utf8"))
    .split("\n")
    .filter((line) => !/^\s*<(p|h1|a|\/p)[\s>]/.test(line))
    .join("\n")
    .trim();
  const examples = (await fs.readFile(path.join(plugin, "tests/upstream/sdk-examples.md"), "utf8")).replace(/^<!--[\s\S]*?-->\s*/, "");
  const full = `${llms}\n# README.md\n\n${readme}\n\n# Examples (vestra-player-sdk v${pkg.version}, examples/)\n\nIn your own project the dependency is "@vestra-player/sdk": "^${pkg.version}" instead of "file:../..".\n\n${examples}`;
  await fs.mkdir(path.join(out, "knowledge"), { recursive: true });
  await fs.writeFile(path.join(out, "knowledge/llms.txt"), llms);
  await fs.writeFile(path.join(out, "knowledge/llms-full.txt"), full);
}

/** Writes vendor/ and knowledge/ under `out` (a plugin-shaped directory). */
export async function vendor(out) {
  const from = sources();
  for (const rel of ["vendor/@vestra-player", "vendor/typescript", "vendor/@types", "vendor/undici-types", "knowledge"])
    await fs.rm(path.join(out, rel), { recursive: true, force: true });
  await copyTree(from.sdk, path.join(out, "vendor/@vestra-player/sdk"), (name) => SDK_FILES.includes(name));
  await copyTree(from.typescript, path.join(out, "vendor/typescript"), (name) => ["package.json", "LICENSE.txt", "ThirdPartyNoticeText.txt", "lib"].includes(name));
  // lib/: only what tsc and transpileModule load (no tsserver, no localized messages).
  const lib = path.join(out, "vendor/typescript/lib");
  for (const item of await fs.readdir(lib, { withFileTypes: true }))
    if (item.isDirectory() || !TS_FILE(item.name)) await fs.rm(path.join(lib, item.name), { recursive: true, force: true });
  await copyTree(from.node, path.join(out, "vendor/@types/node"));
  await copyTree(from.undici, path.join(out, "vendor/undici-types"));
  if (!existsSync(path.join(from.sdk, "llms.txt"))) await knowledge(from.sdk, out);
}

const TREES = ["vendor/@vestra-player", "vendor/typescript", "vendor/@types", "vendor/undici-types", "knowledge"];

async function digests(root) {
  const out = new Map();
  const walk = async (rel) => {
    const full = path.join(root, rel);
    if (!existsSync(full)) return;
    for (const item of await fs.readdir(full, { withFileTypes: true })) {
      const child = path.join(rel, item.name);
      if (item.isDirectory()) await walk(child);
      else out.set(child.split(path.sep).join("/"), createHash("sha256").update(await fs.readFile(path.join(root, child))).digest("hex"));
    }
  };
  for (const rel of TREES) await walk(rel);
  return out;
}

/** Differences between a fresh build and the committed files (empty when current). */
export async function differences() {
  const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-ai-sdk-"));
  try {
    await vendor(scratch);
    const fresh = await digests(scratch);
    const committed = await digests(plugin);
    const names = new Set([...fresh.keys(), ...committed.keys()]);
    return [...names].filter((k) => fresh.get(k) !== committed.get(k)).sort();
  } finally {
    await fs.rm(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes("--write")) {
    await vendor(plugin);
    const { version } = JSON.parse(await fs.readFile(path.join(plugin, "vendor/@vestra-player/sdk/package.json"), "utf8"));
    console.log(`vendor/: @vestra-player/sdk ${version} with its TypeScript and @types/node; knowledge/ ${existsSync(path.join(plugin, "knowledge")) ? "generated" : "from the SDK"}`);
  } else {
    const changed = await differences();
    console.log(changed.length ? `out of date:\n${changed.join("\n")}` : "vendored SDK toolchain is current");
    process.exitCode = changed.length ? 1 : 0;
  }
}
