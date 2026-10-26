#!/usr/bin/env node
/**
 * Builds the built-in plugins a packaged player ships
 * (`Vestra Player.app/Contents/Resources/plugins/`, `<install dir>\resources\plugins\`
 * on Windows). The host runs each in place from its directory; nothing is
 * copied into the user's library (host/src/package_manager.rs `builtin`):
 *
 *   plugins/
 *     builtin.json     { schema: 2, plugins: [{ id, version, directory }] }
 *     <id>/            the content of `vestra-plugin pack` of each plugins/<name>
 *
 * Each plugin is packed into a .vxp as for the store, then unpacked into
 * `<id>/` and the .vxp deleted, so the shipped files are exactly what an
 * install of that .vxp would run. `directory` is always the id.
 *
 * Every directory under plugins/ with a manifest.json is built in (the
 * network-video plugins live in the plugins repository and are installed
 * from the store). Only `vestra.` ids may be built in (the host refuses
 * others). A built-in updates with the player whatever its version.
 *
 *   node scripts/stage-plugins.mjs <destination>/plugins [name...]
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import {
  mkdir,
  readdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { inflateRawSync } from "node:zlib";
import { zipDirectory } from "../packaging/lib.mjs";

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

/** Every file under `dir`, relative, with `/`. */
async function files(dir) {
  return (await readdir(dir, { recursive: true, withFileTypes: true }))
    .filter((item) => item.isFile())
    .map((item) =>
      relative(dir, join(item.parentPath, item.name)).split(sep).join("/"),
    )
    .sort();
}

/**
 * Unpack `archive` into `directory` (new), then check the tree holds
 * exactly the archive's files with the same bytes.
 */
export async function unpack(archive, directory) {
  const { bytes, entries } = await zipDirectory(archive);
  const content = new Map();
  for (const entry of entries) {
    if (entry.name.endsWith("/")) continue;
    const parts = entry.name.split("/");
    if (
      entry.name.includes("\\") ||
      parts.some((part) => part === "" || part === "." || part === "..")
    )
      throw new Error(`${archive}: unsafe entry ${entry.name}`);
    const local = entry.header;
    const start =
      local +
      30 +
      bytes.readUInt16LE(local + 26) +
      bytes.readUInt16LE(local + 28);
    const data = bytes.subarray(start, start + entry.compressed);
    if (entry.method === 0) content.set(entry.name, data);
    else if (entry.method === 8) content.set(entry.name, inflateRawSync(data));
    else
      throw new Error(`${archive}: ${entry.name} uses method ${entry.method}`);
  }
  await mkdir(directory);
  for (const [name, data] of content) {
    const target = join(directory, ...name.split("/"));
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, data, { flag: "wx" });
  }
  const unpacked = await files(directory);
  if (unpacked.join("\n") !== [...content.keys()].sort().join("\n"))
    throw new Error(`${directory} does not match ${archive}`);
  for (const [name, data] of content)
    if (!(await readFile(join(directory, ...name.split("/")))).equals(data))
      throw new Error(`${directory}/${name} differs from ${archive}`);
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
    if (!/^vestra\./i.test(manifest.id))
      throw new Error(`${name}: built-in plugins need a vestra. id`);
    const file = join(destination, `${manifest.id}-${manifest.version}.vxp`);
    // SDK 1.0 named the archive .zip; the content is the same.
    const legacy = join(destination, `${manifest.id}-${manifest.version}.zip`);
    if (!existsSync(file) && existsSync(legacy)) await rename(legacy, file);
    const directory = join(destination, manifest.id);
    await unpack(file, directory);
    await rm(file);
    if (!(await stat(join(directory, "manifest.json"))).isFile())
      throw new Error(`${directory}: no manifest.json`);
    plugins.push({
      id: manifest.id,
      version: manifest.version,
      directory: manifest.id,
    });
  }
  await writeFile(
    join(destination, "builtin.json"),
    `${JSON.stringify({ schema: 2, plugins }, null, 2)}\n`,
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
