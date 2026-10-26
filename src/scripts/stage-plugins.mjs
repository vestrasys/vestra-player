#!/usr/bin/env node
/**
 * Builds the built-in plugins a packaged player ships
 * (`Vestra Player.app/Contents/Resources/plugins/`, `<install dir>\resources\plugins\`
 * on Windows; seeded by the host at
 * start-up; see host/src/package_manager.rs `start_seeding`):
 *
 *   plugins/
 *     builtin.json                 { schema: 1, plugins: [{ id, version, file, sha256 }] }
 *     <id>-<version>.vxp           `vestra-plugin pack` of each plugins/<name>
 *
 * Every directory under plugins/ with a manifest.json is built in (the
 * network-video plugins live in the plugins repository and are installed
 * from the store). A built-in is upgraded on users' machines only when its
 * version here is newer than the one the player last seeded, so bump a
 * plugin's version whenever it changes (`npm run builtin-versions` checks it).
 *
 *   node scripts/stage-plugins.mjs <destination>/plugins [name...]
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import {
  mkdir,
  readdir,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { join, resolve } from "node:path";

const source = resolve(import.meta.dirname, "..");
const pluginsRoot = join(source, "plugins");

export async function builtinNames() {
  return (await readdir(pluginsRoot, { withFileTypes: true }))
    .filter(
      (item) =>
        item.isDirectory() &&
        existsSync(join(pluginsRoot, item.name, "manifest.json")),
    )
    .map((item) => item.name)
    .sort();
}

export async function stagePlugins(destination, names) {
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  const plugins = [];
  for (const name of names ?? (await builtinNames())) {
    const result = spawnSync(
      process.execPath,
      [join(source, "scripts/plugin.mjs"), "pack", name, "--out", destination],
      { stdio: ["ignore", "inherit", "inherit"], cwd: source },
    );
    if (result.status !== 0) throw new Error(`pack ${name} failed`);
    const manifest = JSON.parse(
      await readFile(join(pluginsRoot, name, "dist/manifest.json"), "utf8"),
    );
    const file = `${manifest.id}-${manifest.version}.vxp`;
    // SDK 1.0 named the archive .zip; the content is the same.
    const legacy = join(destination, `${manifest.id}-${manifest.version}.zip`);
    if (!existsSync(join(destination, file)) && existsSync(legacy))
      await rename(legacy, join(destination, file));
    const bytes = await readFile(join(destination, file));
    plugins.push({
      id: manifest.id,
      version: manifest.version,
      file,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    });
  }
  await writeFile(
    join(destination, "builtin.json"),
    `${JSON.stringify({ schema: 1, plugins }, null, 2)}\n`,
  );
  return plugins;
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const [destination, ...names] = process.argv.slice(2);
  if (!destination) {
    console.error(
      "usage: node scripts/stage-plugins.mjs <destination>/plugins [name...]",
    );
    process.exit(2);
  }
  const plugins = await stagePlugins(
    resolve(destination),
    names.length ? names : undefined,
  );
  console.log(`staged ${plugins.length} built-in plugins in ${destination}`);
}
