#!/usr/bin/env node
/**
 * Windows package (packaging-proposal §4, layout §4.2): a staged install
 * directory, a portable zip of it, and an Inno Setup installer
 * (windows/vestra.iss; per-user, no administrator rights):
 *
 *   <install dir>\
 *     vestra-host.exe
 *     libmpv-2.dll
 *     node\node.exe
 *     ffmpeg\ffmpeg.exe, ffprobe.exe, *.dll
 *     resources\ui\, runtime\, plugins\
 *     licenses\
 *
 *   node packaging/bundle-windows.mjs --out <dir>
 *     [--binary <vestra-host.exe>]   default $CARGO_TARGET_DIR/release/vestra-host.exe
 *     [--deps <dir>]                 the Windows deps directory; default $VESTRA_DEPS_DIR
 *     [--rust-licenses <html>]       pre-generated cargo-about output
 *     [--no-installer]               stage and zip only (any OS; iscc is Windows-only)
 *
 * Output in <out>: VestraPlayer-<version>-windows-x64\ (staged),
 * VestraPlayer-<version>-windows-x64-portable.zip and
 * VestraPlayer-<version>-windows-x64-setup.exe.
 */
import { spawnSync } from "node:child_process";
import { readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fetchNode } from "./fetch-node.mjs";
import { COMMITTED_ICO } from "./icon.mjs";
import {
  clean,
  copy,
  directorySize,
  exists,
  fail,
  hostVersion,
  megabytes,
  packaging,
  parseArgs,
  run,
  source,
} from "./lib.mjs";
import {
  artifactBase,
  rustLicenses,
  stageLicenses,
  stageResources,
} from "./stage.mjs";

/** Inno Setup AppId, derived from the macOS bundle id. */
export const APP_ID = "com.vestraplayer.player";

const options = parseArgs(process.argv.slice(2), ["no-installer"]);
if (!options.out)
  fail(
    "usage: bundle-windows.mjs --out <dir> [--binary …] [--deps …] (see the header)",
  );
const out = resolve(options.out);
const binary = resolve(
  options.binary ??
    join(
      process.env.CARGO_TARGET_DIR || join(source, "host/target"),
      "release/vestra-host.exe",
    ),
);
const deps = resolve(
  options.deps ??
    process.env.VESTRA_DEPS_DIR ??
    fail("--deps or VESTRA_DEPS_DIR is required"),
);
if (!exists(binary))
  fail(`${binary} does not exist (cargo build --release first)`);
if (!exists(join(deps, "components.json")))
  fail(`${deps} is not a deps directory (no components.json)`);

const version = await hostVersion();
const base = await artifactBase("windows-x64");
const stage = join(out, base);
await clean(stage);

await copy(binary, join(stage, "vestra-host.exe"));
await copy(join(deps, "libmpv-2.dll"), join(stage, "libmpv-2.dll"));
for (const file of await readdir(join(deps, "ffmpeg")))
  await copy(join(deps, "ffmpeg", file), join(stage, "ffmpeg", file));
const nodeDir = join(tmpdir(), `vestra-node-${process.pid}`);
await fetchNode("win-x64", nodeDir, join(tmpdir(), "vestra-node-cache"));
await copy(join(nodeDir, "node.exe"), join(stage, "node/node.exe"));

await stageResources(join(stage, "resources"));
let rustHtml = options["rust-licenses"] && resolve(options["rust-licenses"]);
if (!rustHtml) {
  rustHtml = join(nodeDir, "rust-crates.html");
  rustLicenses("x86_64-pc-windows-msvc", rustHtml);
}
await stageLicenses(join(stage, "licenses"), {
  rustHtml,
  nodeLicense: join(nodeDir, "LICENSE"),
  deps,
});
await rm(nodeDir, { recursive: true, force: true });

run(process.execPath, [
  join(packaging, "check-licenses.mjs"),
  "--bundle",
  stage,
  "--platform",
  "windows",
]);
console.log(`${stage}: ${megabytes(await directorySize(stage))}`);

// Portable zip: the staged directory as one top-level folder.
const zip = join(out, `${base}-portable.zip`);
await rm(zip, { force: true });
if (!spawnSync("7z", ["i"], { stdio: "ignore" }).error)
  run("7z", ["a", "-tzip", "-mx=9", "-bso0", "-bsp0", zip, stage]);
else run("zip", ["-qr9", zip, base], { cwd: out });

if (!options["no-installer"]) {
  if (process.platform !== "win32")
    fail("the installer needs Inno Setup on Windows (use --no-installer)");
  const iscc =
    [
      process.env.ISCC,
      "C:\\Program Files (x86)\\Inno Setup 6\\ISCC.exe",
      "C:\\Program Files\\Inno Setup 6\\ISCC.exe",
    ].find((path) => path && exists(path)) ?? "iscc";
  const icon = COMMITTED_ICO;
  run(iscc, [
    "/Q",
    `/DAppId=${APP_ID}`,
    `/DAppVersion=${version}`,
    `/DAppVersionNumeric=${version.replace(/[-+].*$/, "")}`,
    `/DSourceDir=${stage}`,
    `/DOutputDir=${out}`,
    `/DOutputBaseFilename=${base}-setup`,
    `/DIconFile=${icon}`,
    join(packaging, "windows/vestra.iss"),
  ]);
}
for (const file of await readdir(out))
  if (file.startsWith(base) && file !== base)
    console.log(`artifact ${join(out, file)}`);
