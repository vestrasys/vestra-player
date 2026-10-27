#!/usr/bin/env node
/**
 * Builds the built-in plugins a packaged player ships
 * (`Vestra Player.app/Contents/Resources/plugins/`, `<install dir>\resources\plugins\`
 * on Windows). The host runs each in place from its directory; nothing is
 * copied into the user's library (host/src/package_manager.rs `builtin`):
 *
 *   plugins/
 *     builtin.json     { schema: 3, plugins: [{ id, version, directory, digest }] }
 *     <id>/            the content of `vestra-plugin pack` of each plugins/<name>
 *
 * Each plugin is packed into a .vxp as for the store, then unpacked into
 * `<id>/` and the .vxp deleted, so the shipped files are exactly what an
 * install of that .vxp would run. `directory` is always the id. `digest` is
 * the package digest of `<id>/` (`packageDigest`, the host's
 * `package_stage::read_tree`): the host runs a built-in in the shared runner
 * only while its directory still has that digest
 * (doc/process/rounds/2026-10-shared-runner.md §3.2).
 *
 * Every directory under plugins/ with a manifest.json is built in (the
 * network-video plugins live in the plugins repository and are installed
 * from the store). Only `vestra.` ids may be built in (the host refuses
 * others). A built-in updates with the player whatever its version.
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

/** The host's `package_stage::DIGEST_DOMAIN`. */
const DIGEST_DOMAIN = Buffer.from("vestra.package-stage/v1\0", "latin1");

function lengthLE(length) {
  const bytes = Buffer.alloc(8);
  bytes.writeBigUInt64LE(BigInt(length));
  return bytes;
}

/**
 * The package digest of a package's files (`Map` of `/`-joined relative
 * name → bytes), as the host computes it (`package_stage::read_tree`):
 * SHA-256 over the domain, then for each file in byte order of its name
 * `len(name)` (u64 LE), the name, `len(bytes)` (u64 LE), the bytes; lowercase
 * hex. `tests/sdk/package-digest.test.ts` and the host's
 * `digest_matches_the_shared_fixture` check both against one fixture
 * (`tests/fixtures/package-digest.json`).
 */
export function packageDigest(files) {
  const names = [...files.keys()]
    .map((name) => Buffer.from(name, "utf8"))
    .sort(Buffer.compare);
  const hash = createHash("sha256");
  hash.update(DIGEST_DOMAIN);
  for (const name of names) {
    const bytes = files.get(name.toString("utf8"));
    hash.update(lengthLE(name.length));
    hash.update(name);
    hash.update(lengthLE(bytes.length));
    hash.update(bytes);
  }
  return hash.digest("hex");
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
 * exactly the archive's files with the same bytes. Returns them (name →
 * bytes).
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
  return content;
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
    const content = await unpack(file, directory);
    await rm(file);
    if (!(await stat(join(directory, "manifest.json"))).isFile())
      throw new Error(`${directory}: no manifest.json`);
    plugins.push({
      id: manifest.id,
      version: manifest.version,
      directory: manifest.id,
      digest: packageDigest(content),
    });
  }
  await writeFile(
    join(destination, "builtin.json"),
    `${JSON.stringify({ schema: 3, plugins }, null, 2)}\n`,
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
