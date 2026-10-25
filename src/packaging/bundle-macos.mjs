#!/usr/bin/env node
/**
 * Vestra Player.app and its DMG (packaging-proposal §3, layout §3.1):
 *
 *   Vestra Player.app/Contents/
 *     Info.plist, PkgInfo
 *     MacOS/vestra-host            the release binary (CFBundleExecutable)
 *     MacOS/node                   Node pinned in node.json
 *     MacOS/ffmpeg, MacOS/ffprobe  from the deps archive
 *     Frameworks/*.dylib           libmpv closure, install names @rpath/<name>
 *     Resources/AppIcon.icns
 *     Resources/ui/, runtime/, plugins/, licenses/
 *
 * Every Mach-O is signed inside out: ad-hoc (`-`) by default so the app runs
 * on the build machine and CI; with --identity, a Developer ID signature
 * with the Hardened Runtime, a timestamp and the entitlements in macos/.
 * The license check runs on the finished bundle.
 *
 *   node packaging/bundle-macos.mjs --out <dir>
 *     [--binary <vestra-host>]   default $CARGO_TARGET_DIR/release/vestra-host
 *                                (else src/host/target/release/vestra-host)
 *     [--deps <dir>]             the macOS deps directory; default $VESTRA_DEPS_DIR
 *     [--rust-licenses <html>]   pre-generated cargo-about output
 *     [--identity <name>]        codesign identity (default ad-hoc)
 *     [--no-dmg] [--zip]         skip the DMG / also zip the .app (ditto)
 *
 * Output: <out>/Vestra Player.app, <out>/VestraPlayer-<version>-macos-arm64.dmg
 * (and .app.zip with --zip).
 */
import { chmod, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fetchNode } from "./fetch-node.mjs";
import { DISPLAY_NAME, infoPlist } from "./macos-plist.mjs";
import { writeIcons } from "./icon.mjs";
import {
  capture,
  clean,
  copy,
  directorySize,
  exists,
  fail,
  hostVersion,
  megabytes,
  packaging,
  parseArgs,
  readJson,
  run,
  source,
} from "./lib.mjs";
import {
  artifactBase,
  rustLicenses,
  stageLicenses,
  stageResources,
} from "./stage.mjs";

/** Floor for LSMinimumSystemVersion (picture in picture needs 14). */
const MIN_MACOS = "14.0";

const options = parseArgs(process.argv.slice(2), ["no-dmg", "zip"]);
if (!options.out)
  fail(
    "usage: bundle-macos.mjs --out <dir> [--binary …] [--deps …] (see the header)",
  );
const out = resolve(options.out);
const binary = resolve(
  options.binary ??
    join(
      process.env.CARGO_TARGET_DIR || join(source, "host/target"),
      "release/vestra-host",
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
const app = join(out, `${DISPLAY_NAME}.app`);
const contents = join(app, "Contents");
await rm(app, { recursive: true, force: true });
await clean(join(contents, "MacOS"));

// ------------------------------------------------------------ binaries

await copy(binary, join(contents, "MacOS/vestra-host"));
await chmod(join(contents, "MacOS/vestra-host"), 0o755);
for (const tool of ["ffmpeg", "ffprobe"])
  await copy(join(deps, "bin", tool), join(contents, "MacOS", tool));
for (const file of await readdir(join(deps, "lib")))
  if (file.endsWith(".dylib") && file !== "libmpv.dylib")
    await copy(join(deps, "lib", file), join(contents, "Frameworks", file));

const nodeDir = join(tmpdir(), `vestra-node-${process.pid}`);
await fetchNode("darwin-arm64", nodeDir, join(tmpdir(), "vestra-node-cache"));
await copy(join(nodeDir, "node"), join(contents, "MacOS/node"));

// The host links libmpv by whatever install name the build saw (an absolute
// Homebrew path on a developer machine): point it into Frameworks/.
const host = join(contents, "MacOS/vestra-host");
const change = [];
for (const reference of references(host))
  if (
    /\/libmpv\.\d+\.dylib$/.test(reference) &&
    !reference.startsWith("@rpath/")
  )
    change.push("-change", reference, `@rpath/${reference.split("/").pop()}`);
for (const rpath of rpaths(host)) change.push("-delete_rpath", rpath);
change.push("-add_rpath", "@executable_path/../Frameworks");
run("install_name_tool", [...change, host]);

// Closure: every non-system reference of every Mach-O resolves in Frameworks/.
const frameworks = new Set(await readdir(join(contents, "Frameworks")));
const machOs = [
  ...["vestra-host", "ffmpeg", "ffprobe", "node"].map((file) =>
    join(contents, "MacOS", file),
  ),
  ...[...frameworks].map((file) => join(contents, "Frameworks", file)),
];
for (const file of machOs)
  for (const reference of references(file)) {
    if (reference.startsWith("/usr/lib/") || reference.startsWith("/System/"))
      continue;
    const name = reference.replace(/^@rpath\//, "");
    if (!reference.startsWith("@rpath/") || !frameworks.has(name))
      fail(`${file} references ${reference}, which is not in Frameworks/`);
  }

// ------------------------------------------------------------ resources

const components = await readJson(join(deps, "components.json"));
const minimum = newer(components.min_macos ?? "0", MIN_MACOS)
  ? components.min_macos
  : MIN_MACOS;
await writeFile(join(contents, "Info.plist"), infoPlist(version, minimum));
await writeFile(join(contents, "PkgInfo"), "APPL????");
await writeIcons({ icns: join(contents, "Resources/AppIcon.icns") });
await stageResources(join(contents, "Resources"));
let rustHtml = options["rust-licenses"] && resolve(options["rust-licenses"]);
if (!rustHtml) {
  rustHtml = join(nodeDir, "rust-crates.html");
  rustLicenses("aarch64-apple-darwin", rustHtml);
}
await stageLicenses(join(contents, "Resources/licenses"), {
  rustHtml,
  nodeLicense: join(nodeDir, "LICENSE"),
  deps,
});
await rm(nodeDir, { recursive: true, force: true });

// ------------------------------------------------------------ signing

const identity = options.identity ?? "-";
const signed = identity !== "-";
const sign = (path, entitlements) =>
  run(
    "codesign",
    [
      "--force",
      "--sign",
      identity,
      ...(signed ? ["--options", "runtime", "--timestamp"] : []),
      ...(entitlements
        ? ["--entitlements", join(packaging, "macos", entitlements)]
        : []),
      path,
    ],
    { quiet: true },
  );
for (const file of frameworks) sign(join(contents, "Frameworks", file));
sign(join(contents, "MacOS/ffmpeg"));
sign(join(contents, "MacOS/ffprobe"));
sign(join(contents, "MacOS/node"), "node.entitlements");
sign(host, "host.entitlements");
sign(app, "host.entitlements");
run("codesign", ["--verify", "--deep", "--strict", "--verbose=1", app]);

run(process.execPath, [
  join(packaging, "check-licenses.mjs"),
  "--bundle",
  contents,
  "--platform",
  "macos",
]);
console.log(
  `${app}: ${megabytes(await directorySize(app))}, LSMinimumSystemVersion ${minimum}`,
);

// ------------------------------------------------------------ DMG / zip

const base = await artifactBase("macos-arm64");
if (options.zip) {
  const zip = join(out, `${base}.app.zip`);
  await rm(zip, { force: true });
  run("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", app, zip]);
}
if (!options["no-dmg"]) {
  const stage = join(out, ".dmg-stage");
  await clean(stage);
  run("ditto", [app, join(stage, `${DISPLAY_NAME}.app`)]);
  await symlink("/Applications", join(stage, "Applications"));
  const dmg = join(out, `${base}.dmg`);
  await rm(dmg, { force: true });
  run("hdiutil", [
    "create",
    "-volname",
    DISPLAY_NAME,
    "-srcfolder",
    stage,
    "-format",
    "ULMO",
    "-fs",
    "HFS+",
    "-ov",
    dmg,
  ]);
  await rm(stage, { recursive: true, force: true });
  if (signed) sign(dmg);
}
for (const file of await readdir(out))
  if (file.startsWith(base)) console.log(`artifact ${join(out, file)}`);

// ------------------------------------------------------------ helpers

function references(file) {
  const lines = capture("otool", ["-L", file]).split("\n").slice(1);
  const id = capture("otool", ["-D", file]).split("\n")[1]?.trim();
  return lines
    .map((line) => line.trim().replace(/ \(compatibility.*\)$/, ""))
    .filter((line) => line && line !== id);
}

function rpaths(file) {
  const lines = capture("otool", ["-l", file]).split("\n");
  const found = [];
  lines.forEach((line, index) => {
    if (line.trim() === "cmd LC_RPATH") {
      const path = lines
        .slice(index, index + 4)
        .find((item) => item.trim().startsWith("path "));
      found.push(path.trim().replace(/^path (.*) \(offset \d+\)$/, "$1"));
    }
  });
  return found;
}

function newer(a, b) {
  const [x, y] = [a, b].map((value) => value.split(".").map(Number));
  for (let index = 0; index < 3; index++)
    if ((x[index] ?? 0) !== (y[index] ?? 0))
      return (x[index] ?? 0) > (y[index] ?? 0);
  return false;
}
