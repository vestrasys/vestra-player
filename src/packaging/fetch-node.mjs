#!/usr/bin/env node
/**
 * The Node.js the player ships:
 * the official nodejs.org binary pinned in node.json.
 *
 *   node packaging/fetch-node.mjs --platform darwin-arm64|win-x64 --out <dir>
 *       Downloads the archive, checks SHASUMS256.txt with gpgv against the
 *       Node release keyring pinned in node.json, checks that SHASUMS and
 *       node.json agree on the archive's SHA-256 and that the archive has it,
 *       then writes <dir>/node (node.exe on Windows) and <dir>/LICENSE.
 *   node packaging/fetch-node.mjs --pin <version>
 *       Verifies that release's SHASUMS256.txt the same way and writes its
 *       hashes into node.json (the way to upgrade Node; keep the major).
 *
 * Downloads are cached in --cache (default <system temp>/vestra-node-cache).
 */
import { spawnSync } from "node:child_process";
import {
  chmod,
  mkdir,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import {
  download,
  exists,
  fail,
  packaging,
  parseArgs,
  readJson,
  reportErrors,
  run,
  sha256File,
  verifySha256,
  zipRead,
} from "./lib.mjs";

const ARCHIVES = {
  "darwin-arm64": (version) => `node-v${version}-darwin-arm64.tar.xz`,
  "win-x64": (version) => `node-v${version}-win-x64.zip`,
};
const pinFile = join(packaging, "node.json");

/** SHASUMS256.txt of a release, GPG-verified; returns { file → sha256 }. */
async function verifiedSums(version, keyring, cache) {
  const base = `https://nodejs.org/dist/v${version}`;
  const dir = join(cache, `v${version}`);
  await mkdir(dir, { recursive: true });
  const kbx = join(cache, `pubring-${keyring.sha256.slice(0, 12)}.kbx`);
  if (!exists(kbx) || (await sha256File(kbx)) !== keyring.sha256)
    await download(keyring.url, kbx);
  await verifySha256(kbx, keyring.sha256);
  const sums = join(dir, "SHASUMS256.txt");
  await download(`${base}/SHASUMS256.txt`, sums);
  await download(`${base}/SHASUMS256.txt.sig`, `${sums}.sig`);
  // Paths relative to the cache, with "./": gpgv looks a bare keyring name
  // up in ~/.gnupg, and the MSYS2 gpg of Git for Windows reads "C:\\…" as a
  // URL. Without gpgv (some Git for Windows builds), gpg --verify with only
  // that keyring and a throw-away home.
  const rel = (path) => `./${relative(cache, path).split(sep).join("/")}`;
  const verify = [`${rel(sums)}.sig`, rel(sums)];
  const gpgv = spawnSync("gpgv", ["--version"], { stdio: "ignore" });
  if (!gpgv.error)
    run("gpgv", ["--keyring", rel(kbx), ...verify], { cwd: cache });
  else {
    await mkdir(join(cache, "gnupg-home"), { recursive: true });
    run(
      "gpg",
      [
        "--batch",
        "--homedir",
        "./gnupg-home",
        "--no-default-keyring",
        "--keyring",
        rel(kbx),
        "--verify",
        ...verify,
      ],
      { cwd: cache },
    );
  }
  const table = {};
  for (const line of (await readFile(sums, "utf8")).split("\n")) {
    const match = /^([0-9a-f]{64})\s+(\S+)$/.exec(line.trim());
    if (match) table[match[2]] = match[1];
  }
  return table;
}

async function pin(version, cache) {
  const current = await readJson(pinFile);
  const sums = await verifiedSums(version, current.keyring, cache);
  const sha256 = {};
  for (const [platform, name] of Object.entries(ARCHIVES)) {
    sha256[platform] = sums[name(version)];
    if (!sha256[platform]) fail(`SHASUMS256.txt has no ${name(version)}`);
  }
  const next = { ...current, version, sha256 };
  await writeFile(pinFile, `${JSON.stringify(next, null, 2)}\n`);
  console.log(`node.json pinned to ${version}`);
}

async function fetchNode(platform, out, cache) {
  const pins = await readJson(pinFile);
  const name = ARCHIVES[platform]?.(pins.version);
  if (!name) fail(`unknown platform ${platform} (${Object.keys(ARCHIVES)})`);
  const expected = pins.sha256[platform];
  const sums = await verifiedSums(pins.version, pins.keyring, cache);
  if (sums[name] !== expected)
    fail(`node.json and the signed SHASUMS256.txt disagree on ${name}`);
  const archive = join(cache, `v${pins.version}`, name);
  if (!exists(archive) || (await sha256File(archive)) !== expected)
    await download(`https://nodejs.org/dist/v${pins.version}/${name}`, archive);
  await verifySha256(archive, expected);
  await mkdir(out, { recursive: true });
  const top = name.replace(/\.(tar\.xz|zip)$/, "");
  if (platform.startsWith("win")) {
    for (const [entry, file] of [
      ["node.exe", "node.exe"],
      ["LICENSE", "LICENSE"],
    ]) {
      const bytes = await zipRead(archive, `${top}/${entry}`);
      if (!bytes) fail(`${name} has no ${top}/${entry}`);
      await writeFile(join(out, file), bytes);
    }
  } else {
    run("tar", [
      "-xJf",
      archive,
      "-C",
      out,
      "--strip-components",
      "1",
      `${top}/bin/node`,
      `${top}/LICENSE`,
    ]);
    await rename(join(out, "bin/node"), join(out, "node"));
    await rm(join(out, "bin"), { recursive: true });
    await chmod(join(out, "node"), 0o755);
  }
  console.log(`Node ${pins.version} (${platform}) verified into ${out}`);
  return pins.version;
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  reportErrors();
  const options = parseArgs(process.argv.slice(2));
  const cache = resolve(options.cache ?? join(tmpdir(), "vestra-node-cache"));
  if (options.pin) await pin(options.pin, cache);
  else if (options.platform && options.out)
    await fetchNode(options.platform, resolve(options.out), cache);
  else
    fail(
      "usage: fetch-node.mjs --platform <darwin-arm64|win-x64> --out <dir> | --pin <version>",
    );
}

export { fetchNode };
