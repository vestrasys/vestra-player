#!/usr/bin/env node
/**
 * Repository shortcut for the SDK's `vestra-plugin` CLI:
 *
 *   node scripts/plugin.mjs <check|build|test|pack> <name...|--all> [--version x.y.z] [--out dir]
 *
 * A name is a directory under plugins/; a name with a slash is a path relative
 * to src/ (for example `examples/bookmark`). `--all` covers plugins/ only.
 * Everything else, including the options, is the CLI's (see its header in
 * @vestra-player/sdk/bin/vestra-plugin.mjs).
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
const pluginsRoot = join(source, "plugins");

const args = process.argv.slice(2);
const command = args.shift();
const options = [];
for (const name of ["--version", "--out"]) {
  const index = args.indexOf(name);
  if (index >= 0) options.push(...args.splice(index, 2));
}
if (!command || !args.length) {
  console.error(
    "usage: node scripts/plugin.mjs <check|build|test|pack> <name...|--all> [--version x.y.z] [--out dir]",
  );
  process.exit(2);
}
const names = args.includes("--all")
  ? (await readdir(pluginsRoot, { withFileTypes: true }))
      .filter(
        (item) =>
          item.isDirectory() &&
          existsSync(join(pluginsRoot, item.name, "manifest.json")),
      )
      .map((item) => item.name)
      .sort()
  : args;
const directories = names.map((name) =>
  name.includes("/") ? resolve(source, name) : join(pluginsRoot, name),
);

// The CLI as the workspace links it (node_modules/@vestra-player/sdk).
const sdkPackage = createRequire(join(source, "package.json")).resolve(
  "@vestra-player/sdk/package.json",
);
const { bin } = JSON.parse(await readFile(sdkPackage, "utf8"));
const cli = join(dirname(sdkPackage), bin["vestra-plugin"]);
const result = spawnSync(
  process.execPath,
  [cli, command, ...directories, ...options],
  { stdio: "inherit", cwd: source },
);
process.exit(result.status ?? 1);
