#!/usr/bin/env node
/**
 * Windows package (packaging-proposal §4, layout §4.2): a staged install
 * directory, a portable zip of it, and an Inno Setup installer
 * (windows/vestra.iss; per-user, no administrator rights):
 *
 *   <install dir>\
 *     vestra-host.exe
 *     libmpv-2.dll, vulkan-1.dll   (vulkan-1.dll: libmpv imports it; GPU drivers
 *                                  install it, but not every Windows has one)
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
 *     [--reuse-host]                 a host built from the current inputs, by
 *                                    host.mjs resolveHost(): the host cache,
 *                                    else --binary if --host-commit (the commit
 *                                    it was built from) has the same host input
 *                                    hash, else a published release's, else
 *                                    cargo build --release; with its crate
 *                                    license page
 *     [--host-commit <rev>]          with --reuse-host and --binary, see above
 *     [--no-build]                   with --reuse-host: fail instead of building
 *
 * <version> is the release version (src/release.json), as is the installer's
 * AppVersion; vestra-host.exe's own file version is the host version
 * (src/host/Cargo.toml, embedded by build.rs).
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
import { resolveHost } from "./host.mjs";
import { COMMITTED_ICO } from "./icon.mjs";
import {
  clean,
  copy,
  directorySize,
  exists,
  fail,
  megabytes,
  packaging,
  parseArgs,
  run,
  source,
  versions,
} from "./lib.mjs";
import {
  artifactBase,
  rustLicenses,
  stageLicenses,
  stageResources,
} from "./stage.mjs";

/** Inno Setup AppId, derived from the macOS bundle id. */
export const APP_ID = "com.vestraplayer.player";

const options = parseArgs(process.argv.slice(2), [
  "no-installer",
  "reuse-host",
  "no-build",
]);
if (!options.out)
  fail(
    "usage: bundle-windows.mjs --out <dir> [--binary …] [--deps …] (see the header)",
  );
if (options["host-commit"] && !(options["reuse-host"] && options.binary))
  fail("--host-commit needs --reuse-host and --binary");
const out = resolve(options.out);
const { release: version } = await versions();
let binary = resolve(
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
if (!exists(join(deps, "components.json")))
  fail(`${deps} is not a deps directory (no components.json)`);
let reusedLicenses;
if (options["reuse-host"]) {
  const host = await resolveHost({
    target: "x86_64-pc-windows-msvc",
    binary: options.binary,
    binaryCommit: options["host-commit"],
    build: !options["no-build"],
  });
  binary = host.binary;
  reusedLicenses = host.licenses;
}
if (!exists(binary))
  fail(`${binary} does not exist (cargo build --release first)`);

const base = await artifactBase("windows-x64");
const stage = join(out, base);
await clean(stage);

await copy(binary, join(stage, "vestra-host.exe"));
// libmpv-2.dll and the DLLs it needs (vulkan-1.dll), from the deps root.
for (const file of await readdir(deps))
  if (file.toLowerCase().endsWith(".dll"))
    await copy(join(deps, file), join(stage, file));
for (const file of await readdir(join(deps, "ffmpeg")))
  await copy(join(deps, "ffmpeg", file), join(stage, "ffmpeg", file));
const nodeDir = join(tmpdir(), `vestra-node-${process.pid}`);
await fetchNode("win-x64", nodeDir, join(tmpdir(), "vestra-node-cache"));
await copy(join(nodeDir, "node.exe"), join(stage, "node/node.exe"));

await stageResources(join(stage, "resources"));
let rustHtml =
  (options["rust-licenses"] && resolve(options["rust-licenses"])) ||
  reusedLicenses;
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

// Portable zip: the staged directory as one top-level folder. Deflate at
// -mx=5, not 9: on 445 MB of libraries 9 took 8x as long (89 s against 11 s
// on an M-series Mac; 215 s on the Windows runner) for a 1.5% smaller zip.
const zip = join(out, `${base}-portable.zip`);
await rm(zip, { force: true });
// Without 7-Zip: Windows has no `zip`, but its bsdtar writes zip archives
// (by full path: Git for Windows' GNU tar, if first on PATH, cannot).
if (!spawnSync("7z", ["i"], { stdio: "ignore" }).error)
  run("7z", ["a", "-tzip", "-mx=5", "-bso0", "-bsp0", zip, stage]);
else if (process.platform === "win32")
  run(
    join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe"),
    ["-a", "-c", "-f", zip, base],
    { cwd: out },
  );
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
