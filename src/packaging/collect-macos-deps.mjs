#!/usr/bin/env node
/**
 * deps.yml (macOS): collects libmpv, ffmpeg and ffprobe from an installed
 * Homebrew with their whole dylib closure into a relocatable directory
 * (packaging-proposal §3.3, §6):
 *
 *   <out>/
 *     lib/*.dylib              install names @rpath/<name>, references
 *                              @rpath/<name>, rpath @loader_path
 *     lib/libmpv.dylib         symlink to libmpv.2.dylib (rsmpv-sys links -lmpv)
 *     bin/ffmpeg, bin/ffprobe  rpaths @executable_path/../Frameworks (the .app)
 *                              and @executable_path/../lib (this directory)
 *     include/mpv/*.h
 *     licenses/<formula>/…     license files from each formula's keg
 *     components.json          formulae with versions, licenses, sources,
 *                              files, and min_macos (the highest minos)
 *
 * Everything is ad-hoc signed (install_name_tool invalidates signatures and
 * arm64 refuses unsigned code). With --sources <dir> the source archive of
 * every formula is fetched (`brew fetch -s`) into <dir> for the GPL source
 * offer, and its SHA-256 recorded.
 *
 *   node packaging/collect-macos-deps.mjs --out <dir> [--sources <dir>]
 */
import {
  chmod,
  copyFile,
  mkdir,
  readdir,
  realpath,
  symlink,
  writeFile,
} from "node:fs/promises";
import { statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import {
  capture,
  clean,
  copy,
  exists,
  fail,
  parseArgs,
  reportErrors,
  run,
  sha256File,
} from "./lib.mjs";

reportErrors();
const options = parseArgs(process.argv.slice(2));
if (!options.out)
  fail("usage: collect-macos-deps.mjs --out <dir> [--sources <dir>]");
const out = resolve(options.out);
const prefix = capture("brew", ["--prefix"]).trim();
const cellar = capture("brew", ["--cellar"]).trim();

const isSystem = (path) =>
  path.startsWith("/usr/lib/") || path.startsWith("/System/");

/** LC_RPATH entries and LC_LOAD_DYLIB-style references of a Mach-O file. */
function machO(path) {
  const loads = capture("otool", ["-l", path]).split("\n");
  const rpaths = [];
  let minos = null;
  for (let index = 0; index < loads.length; index++) {
    const line = loads[index].trim();
    if (line === "cmd LC_RPATH") {
      const entry = loads
        .slice(index, index + 4)
        .find((item) => item.trim().startsWith("path "));
      rpaths.push(entry.trim().replace(/^path (.*) \(offset \d+\)$/, "$1"));
    }
    const match = /^minos (\S+)$/.exec(line);
    if (match) minos = match[1];
  }
  const id = capture("otool", ["-D", path]).split("\n")[1]?.trim() || null;
  const references = capture("otool", ["-L", path])
    .split("\n")
    .slice(1)
    .map((line) => line.trim().replace(/ \(compatibility.*\)$/, ""))
    .filter((line) => line && line !== id);
  return { rpaths, references, minos, id };
}

/** The real file a reference of `from` (a real path) loads. */
function resolveReference(reference, from, info) {
  const loader = dirname(from);
  if (reference.startsWith("@loader_path/"))
    return join(loader, reference.slice("@loader_path/".length));
  if (reference.startsWith("@rpath/")) {
    for (const rpath of info.rpaths) {
      const base = rpath
        .replace(/^@loader_path/, loader)
        .replace(/^@executable_path/, loader);
      const candidate = join(base, reference.slice("@rpath/".length));
      if (exists(candidate)) return candidate;
    }
    throw new Error(
      `${from}: cannot resolve ${reference} (rpaths ${info.rpaths})`,
    );
  }
  return reference;
}

const formulaOf = (real) => {
  if (!real.startsWith(`${cellar}/`))
    throw new Error(`${real} is not in the Homebrew Cellar`);
  const [name, version] = real.slice(cellar.length + 1).split("/");
  return { name, version, keg: join(cellar, name, version) };
};

// ------------------------------------------------------------ closure

const seeds = [
  { path: join(prefix, "opt/mpv/lib/libmpv.2.dylib"), kind: "lib" },
  { path: join(prefix, "opt/ffmpeg/bin/ffmpeg"), kind: "bin" },
  { path: join(prefix, "opt/ffmpeg/bin/ffprobe"), kind: "bin" },
];
const files = new Map(); // real path → { name, kind, info, formula }
const queue = [];
for (const seed of seeds) {
  const real = await realpath(seed.path);
  queue.push({ real, kind: seed.kind, name: basename(seed.path) });
}
while (queue.length) {
  const item = queue.shift();
  if (files.has(item.real)) continue;
  const info = machO(item.real);
  const entry = {
    ...item,
    info,
    formula: formulaOf(item.real),
    deps: new Map(),
  };
  files.set(item.real, entry);
  for (const reference of info.references) {
    if (isSystem(reference)) continue;
    const target = resolveReference(reference, item.real, info);
    const real = await realpath(target);
    const name = basename(target);
    entry.deps.set(reference, { real, name });
    queue.push({ real, kind: "lib", name });
  }
}
// One file name per real file, and no two files with the same name.
const names = new Map();
for (const entry of files.values()) {
  if (entry.kind === "lib" && entry.info.id)
    entry.name = basename(entry.info.id);
  const other = names.get(entry.name);
  if (other && other !== entry.real)
    throw new Error(
      `two libraries named ${entry.name}: ${other}, ${entry.real}`,
    );
  names.set(entry.name, entry.real);
}

// ------------------------------------------------------------ copy + rewrite

await clean(out);
await mkdir(join(out, "lib"));
await mkdir(join(out, "bin"));
let minos = "0";
const newer = (a, b) => {
  const [x, y] = [a, b].map((value) => value.split(".").map(Number));
  for (let index = 0; index < 3; index++)
    if ((x[index] ?? 0) !== (y[index] ?? 0))
      return (x[index] ?? 0) > (y[index] ?? 0);
  return false;
};
for (const entry of files.values()) {
  const destination = join(out, entry.kind, entry.name);
  await copyFile(entry.real, destination);
  await chmod(destination, 0o755);
  const args = [];
  if (entry.kind === "lib") args.push("-id", `@rpath/${entry.name}`);
  for (const [reference, dep] of entry.deps) {
    const name = basename(files.get(dep.real).name);
    args.push("-change", reference, `@rpath/${name}`);
  }
  for (const rpath of entry.info.rpaths) args.push("-delete_rpath", rpath);
  if (entry.kind === "lib") args.push("-add_rpath", "@loader_path");
  else
    args.push(
      "-add_rpath",
      "@executable_path/../Frameworks",
      "-add_rpath",
      "@executable_path/../lib",
    );
  run("install_name_tool", [...args, destination], {
    quiet: true,
    stdio: ["ignore", "inherit", "pipe"],
  });
  if (entry.info.minos && newer(entry.info.minos, minos))
    minos = entry.info.minos;
}
await symlink("libmpv.2.dylib", join(out, "lib/libmpv.dylib"));
for (const entry of files.values())
  run(
    "codesign",
    ["--force", "--sign", "-", join(out, entry.kind, entry.name)],
    {
      quiet: true,
      stdio: ["ignore", "ignore", "pipe"],
    },
  );

// No reference may point outside the directory any more.
for (const entry of files.values()) {
  const info = machO(join(out, entry.kind, entry.name));
  for (const reference of info.references)
    if (!isSystem(reference) && !reference.startsWith("@rpath/"))
      throw new Error(`${entry.name} still references ${reference}`);
}
await copy(join(prefix, "opt/mpv/include/mpv"), join(out, "include/mpv"));

// ------------------------------------------------------------ components

const formulae = new Map();
for (const entry of files.values()) {
  const { name, version, keg } = entry.formula;
  const formula = formulae.get(name) ?? { name, version, keg, files: [] };
  formula.files.push(`${entry.kind}/${entry.name}`);
  formulae.set(name, formula);
}
const info = JSON.parse(
  capture("brew", ["info", "--json=v2", ...formulae.keys()]),
).formulae;
// COPYING, LICENSE.md, … and SPDX-named texts such as LGPL-2.1-or-later.txt.
const LICENSE_FILE =
  /^((licen[cs]e|copying|copyright|notice|unlicense|patents)([._-][\w.+-]*)?|(L?GPL|MIT|BSD|Apache|MPL|ISC|Zlib)[-\w.]*\.txt)$/i;
const NOT_TEXT = /\.(cfg|py|sh|json|cmake|in|c|h|rb|pl)$/i;
const components = [];
let missing = 0;
for (const item of info) {
  const formula = formulae.get(item.name);
  const licenseDir = join(out, "licenses", item.name);
  const licenseFiles = [];
  for (const dir of [
    formula.keg,
    join(formula.keg, "share/doc", item.name),
    join(formula.keg, "share/licenses", item.name),
  ]) {
    if (!exists(dir)) continue;
    for (const file of await readdir(dir, { withFileTypes: true })) {
      if (
        !file.isFile() ||
        !LICENSE_FILE.test(file.name) ||
        NOT_TEXT.test(file.name)
      )
        continue;
      if (licenseFiles.includes(file.name)) continue;
      await mkdir(licenseDir, { recursive: true });
      await copyFile(join(dir, file.name), join(licenseDir, file.name));
      licenseFiles.push(file.name);
    }
  }
  const stable = item.urls?.stable ?? {};
  const component = {
    name: item.name,
    version: formula.version,
    license: item.license,
    homepage: item.homepage,
    source: {
      url: stable.url,
      sha256: stable.checksum ?? null,
      tag: stable.tag ?? null,
      revision: stable.revision ?? null,
    },
    files: formula.files.sort(),
    license_files: licenseFiles
      .sort()
      .map((file) => `licenses/${item.name}/${file}`),
  };
  if (options.sources) {
    const dir = resolve(options.sources);
    await mkdir(dir, { recursive: true });
    run("brew", ["fetch", "--build-from-source", "--formula", item.name], {
      quiet: true,
    });
    const cached = capture("brew", [
      "--cache",
      "--build-from-source",
      "--formula",
      item.name,
    ]).trim();
    let file = `${item.name}-${basename(cached).replace(/^[0-9a-f]{64}--/, "")}`;
    // A git checkout (some formulae) is archived as it is.
    if (statSync(cached).isDirectory()) {
      file += ".tar.gz";
      run(
        "tar",
        ["-czf", join(dir, file), "-C", dirname(cached), basename(cached)],
        { quiet: true },
      );
    } else await copyFile(cached, join(dir, file));
    component.source.archive = file;
    component.source.archive_sha256 = await sha256File(join(dir, file));
  }
  // No license text in the keg: take the top-level ones from the source archive.
  if (!licenseFiles.length && component.source.archive) {
    const archive = join(resolve(options.sources), component.source.archive);
    const listed = capture("tar", ["-tf", archive]).split("\n");
    const wanted = listed.filter((path) => {
      const parts = path.split("/").filter(Boolean);
      return (
        parts.length === 2 &&
        LICENSE_FILE.test(parts[1]) &&
        !NOT_TEXT.test(parts[1])
      );
    });
    if (wanted.length) {
      await mkdir(licenseDir, { recursive: true });
      run(
        "tar",
        [
          "-xf",
          archive,
          "-C",
          licenseDir,
          "--strip-components",
          "1",
          ...wanted,
        ],
        { quiet: true },
      );
      for (const path of wanted)
        component.license_files.push(`licenses/${item.name}/${basename(path)}`);
    }
  }
  if (!component.license_files.length) {
    missing++;
    console.warn(`warning: ${item.name} has no license file`);
  }
  components.push(component);
}
components.sort((a, b) => a.name.localeCompare(b.name));
const manifest = {
  schema: 1,
  platform: "macos-arm64",
  generated: new Date().toISOString(),
  homebrew: capture("brew", ["--version"]).split("\n")[0],
  macos: capture("sw_vers", ["-productVersion"]).trim(),
  min_macos: minos,
  components,
};
await writeFile(
  join(out, "components.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
console.log(
  `collected ${files.size} files from ${components.length} formulae into ${out}; min macOS ${minos}; ${missing} formulae without a license file`,
);
