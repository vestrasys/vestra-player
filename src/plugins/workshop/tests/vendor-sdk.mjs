// Builds the plugin toolchain the AI's tools run and the reference the agent
// reads (vendor/), from the SDK the player is built with
// (src/node_modules/@vestra-player/sdk, locked by src/package-lock.json).
//
//   npm run vendor:sdk           rebuild and write into the plugin
//   node tests/vendor-sdk.mjs    rebuild into a temporary directory and list differences
//
// vendor/ is laid out like a node_modules folder without being one (git
// ignores node_modules/): the CLI finds TypeScript through NODE_PATH.
//
//   vendor/@vestra-player/sdk/   package.json, LICENSE, README.md, llms.txt, bin/ (vestra-plugin CLI),
//                                dist/ (manifest rules, TestHost, .d.ts, the docs tree
//                                docs.js + docs-tree.json.gz), slint/ (widget library)
//   vendor/typescript/           the SDK's TypeScript: tsc and the compiler API, lib .d.ts
//   vendor/@types/node/          the SDK's @types/node, and its undici-types
//   vendor/undici-types/
//
// Maintainer's alternative to request R-h: the player's
// runtime does not ship the CLI, so this plugin brings it along.
//
// The reference the agent reads is the SDK's own documentation tree
// (@vestra-player/sdk/docs: dist/docs.js and dist/docs-tree.json.gz, since
// 1.5.50; the agent's docs and docs_search tools), copied with the package
// together with its short llms.txt; an SDK without them is refused.
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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

const TREES = ["vendor/@vestra-player", "vendor/typescript", "vendor/@types", "vendor/undici-types"];
const SDK_FILES = ["package.json", "LICENSE", "README.md", "bin", "dist", "slint", "llms.txt", "capability-scan.json"];
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

/** Writes vendor/ under `out` (a plugin-shaped directory). */
export async function vendor(out) {
  const from = sources();
  for (const file of ["llms.txt", "dist/docs.js", "dist/docs-tree.json.gz"])
    if (!existsSync(path.join(from.sdk, file))) throw new Error(`@vestra-player/sdk at ${from.sdk} has no ${file} (needs 1.5.50 or later)`);
  for (const rel of TREES) await fs.rm(path.join(out, rel), { recursive: true, force: true });
  await copyTree(from.sdk, path.join(out, "vendor/@vestra-player/sdk"), (name) => SDK_FILES.includes(name));
  await copyTree(from.typescript, path.join(out, "vendor/typescript"), (name) => ["package.json", "LICENSE.txt", "ThirdPartyNoticeText.txt", "lib"].includes(name));
  // lib/: only what tsc and transpileModule load (no tsserver, no localized messages).
  const lib = path.join(out, "vendor/typescript/lib");
  for (const item of await fs.readdir(lib, { withFileTypes: true }))
    if (item.isDirectory() || !TS_FILE(item.name)) await fs.rm(path.join(lib, item.name), { recursive: true, force: true });
  await copyTree(from.node, path.join(out, "vendor/@types/node"));
  await copyTree(from.undici, path.join(out, "vendor/undici-types"));
}

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
    console.log(`vendor/: @vestra-player/sdk ${version} (with its documentation tree and llms.txt), its TypeScript and @types/node`);
  } else {
    const changed = await differences();
    console.log(changed.length ? `out of date:\n${changed.join("\n")}` : "vendored SDK toolchain is current");
    process.exitCode = changed.length ? 1 : 0;
  }
}
