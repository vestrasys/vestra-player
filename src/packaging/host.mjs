#!/usr/bin/env node
/**
 * The host binary's identity, and reusing an already built host
 * (doc/process/ci.md "Reusing the host binary").
 *
 * The player's release version (src/release.json) and the host binary's
 * version (src/host/Cargo.toml) are separate: a release that changes only
 * built-in plugins, the runtime's staged files or other non-host files
 * packages a host binary that was already built from the same inputs, on CI
 * and locally, instead of compiling it again.
 *
 * The "host input hash" names those inputs: a SHA-256 over a manifest of
 *   - a recipe line (cargo build --release --locked in src/host, default
 *     features, no RUSTFLAGS, the cargo-about version; bump RECIPE when the
 *     build command or the cargo-about version changes),
 *   - the target triple,
 *   - the deps archive the host links (src/packaging/deps.json, that
 *     platform's file and SHA-256: libmpv's import library / dylib),
 *   - every file under src/host/ except those that cannot reach the binary
 *     (EXCLUDE: integration tests, the separate gpu-probe crate, examples,
 *     *.md, lint and format configs, cargo-deny's config), with Cargo.toml
 *     (so the host version), Cargo.lock, .cargo/config.toml, build.rs and
 *     everything it reads,
 *   - every file a host source names in include_str!/include_bytes!/#[path]
 *     wherever it lives (src/assets/fonts/… today), found by scanning,
 *   - rust-toolchain.toml,
 *   - src/packaging/about.toml and about.hbs: the host's crate license page
 *     (licenses/rust-crates.html) is reused with the binary.
 * Files are read as checked out (LF everywhere, .gitattributes), so the same
 * commit gives the same hash on Windows, macOS, Linux and CI. Not covered:
 * the git commit build.rs embeds for `plugin.app` (`build`), which then names
 * the commit the binary was built from (host.json `commit`).
 *
 *   node packaging/host.mjs hash [--target <triple>] [--rev <commit>] [--list]
 *       The hash of the working tree (tracked and untracked, not ignored
 *       files), or of a commit; --list prints the manifest instead.
 *   node packaging/host.mjs versions
 *       Prints release=<x.y.z> and host=<x.y.z>; fails unless release ≥ host.
 *   node packaging/host.mjs check-bump [--target <triple>] [--repo o/n] [--releases N] [--releases-dir <dir>]
 *       For each target (default both): fails when a published release
 *       shipped this host version built from other inputs, or a higher host
 *       version (raise version in src/host/Cargo.toml). Run before tagging;
 *       release.yml's verify job runs it, fetch and --reuse-host check it too.
 *       --releases-dir reads releases from a directory (directoryReleases).
 *   node packaging/host.mjs fetch --target <triple> --out <dir> [--repo o/n] [--releases N]
 *       Looks through the newest N (30) published releases for a host.json
 *       whose entry for this target has this hash (after the check-bump
 *       check; a failure there, or a release that does not check out, is an
 *       error, not "nothing found"); downloads that release's
 *       archive, checks host.json and the archive against its SHA256SUMS and
 *       the binary against host.json, and writes <dir>/vestra-host[.exe],
 *       <dir>/rust-crates.html and <dir>/host.json. found=true|false (and the
 *       paths) go to $GITHUB_OUTPUT when set.
 *   node packaging/host.mjs record --target <triple> --archive <zip> --out <json> [--reused <host.json>]
 *       The release's host.json entry for this target, read from the
 *       packaged archive (portable zip / .app.zip): the release job merges
 *       one per platform into the host.json asset.
 *
 * bundle-windows.mjs / bundle-macos.mjs --reuse-host use resolveHost():
 * the local host cache, then a published release, then cargo build.
 */
import { spawnSync } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, posix, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import {
  capture,
  compareVersions,
  copy,
  exists,
  hostVersion,
  matchesAny,
  parseArgs,
  readJson,
  repo,
  run,
  sha256,
  sha256File,
  source,
  versions,
  zipEntries,
  zipRead,
} from "./lib.mjs";

/**
 * Bump when the build command or the manifest format changes. It names the
 * cargo-about version too: rust-crates.html is reused with the binary, so a
 * new cargo-about (`tool: cargo-about@…` in .github/workflows/_package.yml;
 * a test checks the two agree) must not reuse pages the old one wrote.
 */
export const RECIPE =
  "vestra-host-input/1 cargo build --release --locked (src/host, default features, no RUSTFLAGS); cargo-about 0.9.2";

export const TARGETS = {
  "x86_64-pc-windows-msvc": {
    platform: "windows-x64",
    exe: "vestra-host.exe",
    // VestraPlayer-<version>-windows-x64/vestra-host.exe
    binary: /^[^/]+\/vestra-host\.exe$/,
    licenses: /^[^/]+\/licenses\/rust-crates\.html$/,
    archive: (version) => `VestraPlayer-${version}-windows-x64-portable.zip`,
  },
  "aarch64-apple-darwin": {
    platform: "macos-arm64",
    exe: "vestra-host",
    binary: /^Vestra Player\.app\/Contents\/MacOS\/vestra-host$/,
    licenses:
      /^Vestra Player\.app\/Contents\/Resources\/licenses\/rust-crates\.html$/,
    archive: (version) => `VestraPlayer-${version}-macos-arm64.app.zip`,
  },
};

export function defaultTarget() {
  if (process.platform === "win32" && process.arch === "x64")
    return "x86_64-pc-windows-msvc";
  if (process.platform === "darwin" && process.arch === "arm64")
    return "aarch64-apple-darwin";
  throw new Error(`no packaged host for ${process.platform}-${process.arch}`);
}

function targetInfo(target) {
  const info = TARGETS[target];
  if (!info)
    throw new Error(
      `unknown target ${target} (${Object.keys(TARGETS).join(", ")})`,
    );
  return info;
}

// ------------------------------------------------------------ the hash

const ROOTS = [
  "src/host",
  "rust-toolchain.toml",
  "src/packaging/about.toml",
  "src/packaging/about.hbs",
];

/** Under src/host/ but never part of the binary (conservative: when in doubt, keep a file in). */
const EXCLUDE = [
  "src/host/tests/**", // integration tests: separate test binaries
  "src/host/unsafe/gpu-probe/**", // own crate and lock file, not a dependency, not shipped
  "src/host/**/examples/**",
  "src/host/**/*.md",
  "src/host/**/clippy.toml",
  "src/host/**/rustfmt.toml",
  "src/host/deny.toml",
];

const INCLUDE_MACRO = /include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)/g;
const PATH_ATTRIBUTE = /#\[path\s*=\s*"([^"]+)"\s*\]/g;

/** Reads files from the working tree or from one commit. */
function treeReader(rev) {
  const git = (args, options) =>
    capture("git", args, { cwd: repo, ...options });
  if (!rev) {
    const listed = git([
      "ls-files",
      "-z",
      "--cached",
      "--others",
      "--exclude-standard",
      "--",
      ...ROOTS,
    ])
      .split("\0")
      .filter(Boolean);
    return {
      list: async () =>
        [...new Set(listed)].filter((path) => exists(join(repo, path))),
      read: async (path) =>
        exists(join(repo, path)) ? readFile(join(repo, path)) : undefined,
    };
  }
  const commit = git(["rev-parse", "--verify", `${rev}^{commit}`]).trim();
  const blobs = new Map();
  const tree = (paths) =>
    git(["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...paths])
      .split("\0")
      .filter(Boolean)
      .map((line) => {
        const [meta, path] = line.split("\t");
        const [, type, object] = meta.split(" ");
        return { type, object, path };
      })
      .filter((entry) => entry.type === "blob");
  for (const entry of tree(ROOTS)) blobs.set(entry.path, entry.object);
  // One `git cat-file --batch` for many objects: "<oid> blob <size>\n<bytes>\n".
  const cat = (objects) => {
    const result = spawnSync("git", ["cat-file", "--batch"], {
      cwd: repo,
      input: `${objects.join("\n")}\n`,
      maxBuffer: 1 << 30,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error("git cat-file --batch failed");
    const bytes = result.stdout;
    const out = new Map();
    let offset = 0;
    for (const object of objects) {
      const end = bytes.indexOf(0x0a, offset);
      const [oid, type, size] = bytes.toString("utf8", offset, end).split(" ");
      if (oid !== object || type !== "blob")
        throw new Error(`git cat-file: unexpected ${oid} ${type}`);
      out.set(object, bytes.subarray(end + 1, end + 1 + Number(size)));
      offset = end + 1 + Number(size) + 1;
    }
    return out;
  };
  const loaded = new Map();
  return {
    commit,
    list: async () => {
      for (const [object, bytes] of cat([...new Set(blobs.values())]))
        loaded.set(object, bytes);
      return [...blobs.keys()];
    },
    read: async (path) => {
      if (!blobs.has(path)) {
        const [entry] = tree([path]);
        if (!entry) return undefined;
        blobs.set(path, entry.object);
      }
      const object = blobs.get(path);
      if (!loaded.has(object)) loaded.set(object, cat([object]).get(object));
      return loaded.get(object);
    },
  };
}

/**
 * { hash, manifest, files } for `target`, of the working tree or of `rev`.
 * `manifest` is the text hashed: one line per input.
 */
export async function hostInputs(target = defaultTarget(), rev) {
  const info = targetInfo(target);
  const reader = treeReader(rev);
  const contents = new Map();
  const add = async (path) => {
    if (contents.has(path)) return true;
    const bytes = await reader.read(path);
    if (bytes === undefined) return false;
    contents.set(path, bytes);
    return true;
  };
  for (const path of (await reader.list()).sort())
    if (!matchesAny(path, EXCLUDE)) await add(path);
  // Files the sources pull in, wherever they are (and transitively).
  const queue = [...contents.keys()].filter((path) => path.endsWith(".rs"));
  while (queue.length) {
    const file = queue.shift();
    const text = contents.get(file).toString("utf8");
    for (const [pattern, required] of [
      [INCLUDE_MACRO, true],
      [PATH_ATTRIBUTE, false],
    ])
      for (const [, relative] of text.matchAll(pattern)) {
        const path = posix.normalize(posix.join(posix.dirname(file), relative));
        if (path.startsWith("../"))
          throw new Error(`${file}: ${relative} is outside the repository`);
        const known = contents.has(path);
        if (!(await add(path))) {
          if (required) throw new Error(`${file}: ${relative} does not exist`);
          continue;
        }
        if (!known && path.endsWith(".rs")) queue.push(path);
      }
  }
  const pins = JSON.parse(
    (await reader.read("src/packaging/deps.json"))?.toString("utf8") ?? "{}",
  );
  const deps = pins.assets?.[info.platform];
  if (!deps?.sha256)
    throw new Error(`src/packaging/deps.json pins no ${info.platform} asset`);
  const lines = [
    RECIPE,
    `target ${target}`,
    `deps ${deps.file} ${deps.sha256}`,
    ...[...contents.keys()]
      .sort()
      .map((path) => `file ${sha256(contents.get(path))} ${path}`),
  ];
  const manifest = `${lines.join("\n")}\n`;
  return {
    hash: sha256(manifest),
    manifest,
    files: contents.size,
    commit: reader.commit,
  };
}

export const hostInputHash = async (target, rev) =>
  (await hostInputs(target, rev)).hash;

// ------------------------------------------------------------ releases

function repository(option) {
  if (option) return option;
  if (process.env.GITHUB_REPOSITORY) return process.env.GITHUB_REPOSITORY;
  const url = capture("git", ["remote", "get-url", "origin"], {
    cwd: repo,
  }).trim();
  const match = /github\.com[:/]([^/]+\/[^/]+?)(?:\.git)?$/.exec(url);
  if (!match)
    throw new Error(`origin ${url} is not a GitHub repository (--repo)`);
  return match[1];
}

/** SHA256SUMS → Map(name → hex). */
function parseSums(text) {
  const sums = new Map();
  for (const line of text.split(/\r?\n/)) {
    const match = /^([0-9a-f]{64})\s+\*?(.+)$/.exec(line.trim());
    if (match) sums.set(match[2], match[1]);
  }
  return sums;
}

/**
 * Published releases through gh (GH_TOKEN on CI, the gh login locally):
 * releases() newest first, download(tag, names, dir).
 */
export function githubReleases(repoName, limit = 30) {
  repoName = repository(repoName);
  // gh never waits on CI: no stdin, no prompts, and a time limit (killed,
  // then retried and reported like any other failed lookup).
  const gh = (timeout) => ({
    env: { ...process.env, GH_PROMPT_DISABLED: "1" },
    timeout,
    killSignal: "SIGKILL",
  });
  return {
    name: repoName,
    releases: () =>
      retry(`gh api ${repoName} releases`, () =>
        JSON.parse(
          capture(
            "gh",
            [
              "api",
              `repos/${repoName}/releases?per_page=${limit}`,
              "--jq",
              "[.[] | select(.draft | not) | {tag: .tag_name, assets: [.assets[].name]}]",
            ],
            gh(60_000),
          ),
        ),
      ),
    download: (tag, names, dir) =>
      retry(`gh release download ${tag}`, () =>
        run(
          "gh",
          [
            "release",
            "download",
            tag,
            "--repo",
            repoName,
            ...names.flatMap((name) => ["--pattern", name]),
            "--dir",
            dir,
            "--clobber",
          ],
          {
            quiet: true,
            stdio: ["ignore", "inherit", "inherit"],
            ...gh(600_000),
          },
        ),
      ),
  };
}

/** Runs `action` up to `attempts` times, waiting 2 s, then 4 s, between them. */
export async function retry(what, action, attempts = 3) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await action();
    } catch (error) {
      if (attempt >= attempts) throw error;
      console.warn(`host: ${what} failed (${error.message}); retrying`);
      await new Promise((done) => setTimeout(done, 2000 * attempt));
    }
  }
}

/**
 * Releases from a directory instead of GitHub (offline mirrors, tests):
 * <dir>/releases.json lists the tags newest first, <dir>/<tag>/ holds each
 * release's assets.
 */
export function directoryReleases(dir) {
  return {
    name: dir,
    releases: async () =>
      Promise.all(
        (await readJson(join(dir, "releases.json"))).map(async (tag) => ({
          tag,
          assets: await readdir(join(dir, tag)),
        })),
      ),
    download: async (tag, names, out) => {
      for (const name of names)
        await copy(join(dir, tag, name), join(out, name));
    },
  };
}

/**
 * An error that must stop packaging rather than fall back to a build: a
 * published release whose files do not check out (verification), or a host
 * version that a different host was already released under (bump). Anything
 * else from a release lookup (no gh, no network) only means "nothing to
 * reuse".
 */
export class HostError extends Error {
  constructor(kind, message) {
    super(message);
    this.name = "HostError";
    this.kind = kind;
  }
}

async function checked(path, sums, name) {
  const expected = sums.get(name);
  if (!expected)
    throw new HostError("verification", `SHA256SUMS does not list ${name}`);
  const actual = await sha256File(path);
  if (actual !== expected)
    throw new HostError(
      "verification",
      `${name}: SHA-256 ${actual}, SHA256SUMS says ${expected}`,
    );
}

/**
 * Every published (not draft) release with a host.json, newest first:
 * [{ tag, assets, hosts, sums }], host.json checked against the release's
 * SHA256SUMS. Throws a HostError when one does not check out.
 */
export async function releasedHosts({ from = githubReleases(), scratch }) {
  const releases = (await from.releases()).filter((release) =>
    release.assets.includes("host.json"),
  );
  const out = [];
  for (const release of releases) {
    const dir = join(scratch, release.tag.replace(/[^0-9A-Za-z.+-]/g, "_"));
    await rm(dir, { recursive: true, force: true });
    await mkdir(dir, { recursive: true });
    // A host.json nothing vouches for is not "no release": stop.
    if (!release.assets.includes("SHA256SUMS"))
      throw new HostError(
        "verification",
        `${release.tag} has a host.json but no SHA256SUMS`,
      );
    await from.download(release.tag, ["host.json", "SHA256SUMS"], dir);
    const sums = parseSums(await readFile(join(dir, "SHA256SUMS"), "utf8"));
    try {
      await checked(join(dir, "host.json"), sums, "host.json");
    } catch (error) {
      throw new HostError("verification", `${release.tag}: ${error.message}`);
    }
    const hosts = (await readJson(join(dir, "host.json"))).hosts ?? {};
    out.push({ ...release, hosts, sums, dir });
  }
  return out;
}

/**
 * The host version must name one host: fails (HostError "bump") when a
 * published release shipped a `target` host under `version` that was built
 * from other inputs than `hash`. Two binaries with one version would share
 * everything keyed by it (the GPU crash guard is keyed by host version and
 * driver, so a guard the old binary set would stick to the fixed one).
 * Returns the newest release that shipped this host version, if any.
 */
export function checkHostBump({ target, hash, version, released }) {
  const platform = targetInfo(target).platform;
  const records = released
    .map((release) => ({ release, record: release.hosts[platform] }))
    .filter(({ record }) => record?.target === target);
  // Equal by precedence, not by text: 0.7.3+fix is still 0.7.3 to every
  // version comparison (semver ignores build metadata), so it cannot name a
  // second binary either.
  let same;
  for (const { release, record } of records) {
    if (compareVersions(record.hostVersion, version) !== 0) continue;
    if (record.inputHash !== hash)
      throw new HostError(
        "bump",
        `host ${version} (${platform}) was already released in ${release.tag}${record.hostVersion === version ? "" : ` (as ${record.hostVersion})`} built from other inputs ` +
          `(${record.inputHash}, now ${hash}): raise version in src/host/Cargo.toml ` +
          `(doc/releases/README.md "Release version and host version")`,
      );
    same ??= release;
  }
  if (same) return same;
  // A new host: its version must also be above every one released.
  for (const { release, record } of records) {
    if (compareVersions(record.hostVersion, version) > 0)
      throw new HostError(
        "bump",
        `host ${version} (${platform}) is lower than host ${record.hostVersion} released in ${release.tag}: raise version in src/host/Cargo.toml`,
      );
  }
  return undefined;
}

/**
 * The newest published (not draft) release whose host.json has `hash` for
 * `target`: its binary and rust-crates.html, verified, in `out`. Returns
 * { binary, licenses, record } or undefined. Checks the host version first
 * (checkHostBump). HostError when a release does not check out.
 */
export async function fetchReleasedHost({
  target,
  hash,
  out,
  from = githubReleases(),
  version,
}) {
  const info = targetInfo(target);
  const scratch = join(out, ".download");
  try {
    const released = await releasedHosts({ from, scratch });
    if (!released.length) {
      console.log(`host: no published release of ${from.name} has a host.json`);
      return undefined;
    }
    checkHostBump({
      target,
      hash,
      version: version ?? (await hostVersion()),
      released,
    });
    const release = released.find(
      (item) =>
        item.hosts[info.platform]?.inputHash === hash &&
        item.hosts[info.platform]?.target === target,
    );
    if (!release) {
      console.log(
        `host: none of ${released.map((item) => item.tag).join(", ")} has a ${info.platform} host with input hash ${hash}`,
      );
      return undefined;
    }
    const record = release.hosts[info.platform];
    console.log(
      `host: ${release.tag} has a ${info.platform} host built from these inputs (host ${record.hostVersion}, commit ${record.commit})`,
    );
    const bad = (message) =>
      new HostError("verification", `${release.tag}: ${message}`);
    if (!release.assets.includes(record.archive))
      throw bad(`no ${record.archive}`);
    await from.download(release.tag, [record.archive], release.dir);
    const archive = join(release.dir, record.archive);
    await checked(archive, release.sums, record.archive);
    const binary = await zipRead(archive, record.binary);
    if (!binary) throw bad(`${record.archive} has no ${record.binary}`);
    if (sha256(binary) !== record.binarySha256)
      throw bad(`${record.binary}: SHA-256 differs from host.json`);
    const licenses = await zipRead(archive, record.licenses);
    if (!licenses) throw bad(`${record.archive} has no ${record.licenses}`);
    await writeFile(join(out, info.exe), binary, { mode: 0o755 });
    await writeFile(join(out, "rust-crates.html"), licenses);
    const reused = { ...record, reusedFrom: record.reusedFrom ?? release.tag };
    await writeFile(
      join(out, "host.json"),
      `${JSON.stringify(reused, null, 2)}\n`,
    );
    return {
      binary: join(out, info.exe),
      licenses: join(out, "rust-crates.html"),
      record: reused,
    };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

/** This build's host.json entry for `target`, read from its archive. */
export async function hostRecord({ target, archive, reused }) {
  const info = targetInfo(target);
  const names = (await zipEntries(archive)).filter(
    (name) => !name.endsWith("/"),
  );
  const find = (pattern, what) => {
    const found = names.filter((name) =>
      pattern.test(name.replaceAll("\\", "/")),
    );
    if (found.length !== 1)
      throw new Error(
        `${archive}: ${found.length} entries look like the ${what}`,
      );
    return found[0];
  };
  const binary = find(info.binary, "host binary");
  const licenses = find(info.licenses, "crate license page");
  const inputHash = await hostInputHash(target);
  if (reused && reused.inputHash !== inputHash)
    throw new Error(
      `the reused host's input hash ${reused.inputHash} is not ${inputHash}`,
    );
  return {
    target,
    inputHash,
    hostVersion: await hostVersion(),
    // The commit the binary was built from (its `plugin.app` build).
    commit:
      reused?.commit ??
      capture("git", ["rev-parse", "HEAD"], { cwd: repo }).trim(),
    binarySha256: sha256(await zipRead(archive, binary)),
    archive: basename(archive),
    binary,
    licenses,
    reusedFrom: reused?.reusedFrom ?? null,
  };
}

// ------------------------------------------------------------ local reuse

const cargoTarget = () =>
  resolve(process.env.CARGO_TARGET_DIR || join(source, "host/target"));

/**
 * A host binary for `target` built from the current inputs, for local
 * packaging (bundle-*.mjs --reuse-host). In order:
 *   1. the host cache, $VESTRA_HOST_CACHE or <CARGO_TARGET_DIR>/vestra-host-cache,
 *      one directory per target and hash (every host this resolves lands there);
 *   2. `binary` when `binaryCommit` names the commit it was built from and
 *      that commit's hash is the current one (an older staged build);
 *   3. a published release (gh, logged in, or `from`) with that hash, after
 *      checking that no release shipped this host version built from other
 *      inputs (checkHostBump);
 *   4. cargo build --release --locked in src/host (unless build is false),
 *      in CARGO_TARGET_DIR (one persistent directory keeps it incremental).
 * Step 3 finding nothing leads to 4; a release that does not check out or a
 * host version that needs raising throws (HostError) instead.
 * Returns { binary, licenses (rust-crates.html, made with cargo about once
 * per hash), source }.
 */
export async function resolveHost({
  target = defaultTarget(),
  binary,
  binaryCommit,
  repoName,
  from,
  build = true,
}) {
  const info = targetInfo(target);
  const started = Date.now();
  const hash = await hostInputHash(target);
  console.log(
    `host: input hash ${hash} (${target}, ${Date.now() - started} ms)`,
  );
  const cache = resolve(
    process.env.VESTRA_HOST_CACHE || join(cargoTarget(), "vestra-host-cache"),
    target,
    hash,
  );
  const cachedBinary = join(cache, info.exe);
  const cachedLicenses = join(cache, "rust-crates.html");
  const done = async (how, record) => {
    if (record)
      await writeFile(
        join(cache, "host.json"),
        `${JSON.stringify(record, null, 2)}\n`,
      );
    if (!exists(cachedLicenses)) {
      const { rustLicenses } = await import("./stage.mjs");
      rustLicenses(target, cachedLicenses);
    }
    console.log(`host: ${how} → ${cachedBinary}`);
    return { binary: cachedBinary, licenses: cachedLicenses, source: how };
  };

  if (exists(cachedBinary)) return done("cached");
  await mkdir(cache, { recursive: true });

  if (binary && binaryCommit) {
    const theirs = await hostInputs(target, binaryCommit);
    if (theirs.hash === hash) {
      await copy(resolve(binary), cachedBinary);
      return done(`${binary} (built from ${theirs.commit})`, {
        inputHash: hash,
        commit: theirs.commit,
        from: resolve(binary),
      });
    }
    console.log(
      `host: ${binary} was built from inputs that differ (${theirs.hash}); not reused`,
    );
  }

  // No matching release: build. A release that does not check out, or a host
  // version already released with other inputs: stop (HostError), never
  // build around it. Only an unavailable lookup (no gh login, offline)
  // falls through, with a warning, since then nothing could be checked.
  try {
    const found = await fetchReleasedHost({
      target,
      hash,
      out: cache,
      from: from ?? githubReleases(repoName),
    });
    if (found) return done(`release ${found.record.reusedFrom}`);
  } catch (error) {
    if (error instanceof HostError) throw error;
    console.warn(
      `host: published releases could not be looked up (${error.message}); the host version was not checked against them (host.mjs check-bump)`,
    );
  }

  if (!build)
    throw new Error(`no host built from input hash ${hash}; build it`);
  if (!process.env.CARGO_TARGET_DIR)
    console.warn(
      "host: CARGO_TARGET_DIR is not set; a persistent one (such as <work dir>/target-release) keeps release builds incremental",
    );
  run("cargo", ["build", "--release", "--locked"], {
    cwd: join(source, "host"),
  });
  if ((await hostInputHash(target)) !== hash)
    throw new Error("src/host changed during the build; run again");
  await copy(join(cargoTarget(), "release", info.exe), cachedBinary);
  return done("cargo build", {
    inputHash: hash,
    commit: capture("git", ["rev-parse", "HEAD"], { cwd: repo }).trim(),
  });
}

// ------------------------------------------------------------ CLI

async function output(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  const text = Object.entries(values)
    .map(([key, value]) => {
      if (/[\r\n]/.test(String(value)))
        throw new Error(`${key}: multi-line value`);
      return `${key}=${value}\n`;
    })
    .join("");
  await writeFile(process.env.GITHUB_OUTPUT, text, { flag: "a" });
}

// Not import.meta.main / import.meta.filename: undefined before Node 24.2 /
// 20.11, which would turn every command into a silent exit 0.
if (
  process.argv[1] &&
  pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  const [command, ...rest] = process.argv.slice(2);
  try {
    const options = parseArgs(rest, ["list"]);
    // Not computed up front: `versions` and `check-bump` run on Linux too,
    // where there is no default target.
    const target = () => options.target ?? defaultTarget();
    const limit = Number(options.releases ?? 30);
    if (command === "check-bump") {
      const targets = options.target ? [options.target] : Object.keys(TARGETS);
      const scratch = await mkdtemp(join(tmpdir(), "vestra-check-bump-"));
      try {
        const released = await releasedHosts({
          from: options["releases-dir"]
            ? directoryReleases(resolve(options["releases-dir"]))
            : githubReleases(options.repo, limit),
          scratch,
        });
        const version = await hostVersion();
        for (const triple of targets) {
          const { platform } = targetInfo(triple);
          const hash = await hostInputHash(triple);
          const same = checkHostBump({
            target: triple,
            hash,
            version,
            released,
          });
          const newest = released.find((release) => release.hosts[platform]);
          console.log(
            same
              ? `${platform}: host ${version}, the same inputs as in ${same.tag} (reused)`
              : newest
                ? `${platform}: host ${version} is new (last released: ${newest.hosts[platform].hostVersion} in ${newest.tag})`
                : `${platform}: host ${version}; no published release has a host.json yet`,
          );
        }
      } finally {
        await rm(scratch, { recursive: true, force: true });
      }
    } else if (command === "hash") {
      const inputs = await hostInputs(target(), options.rev);
      console.log(options.list ? inputs.manifest.trimEnd() : inputs.hash);
    } else if (command === "versions") {
      const { release, host } = await versions();
      console.log(`release=${release}\nhost=${host}`);
      await output({ release, host });
    } else if (command === "fetch") {
      if (!options.out) throw new Error("fetch needs --out <dir>");
      const out = resolve(options.out);
      await mkdir(out, { recursive: true });
      const hash = await hostInputHash(target());
      console.log(`host input hash (${target()}): ${hash}`);
      const found = await fetchReleasedHost({
        target: target(),
        hash,
        out,
        from: githubReleases(options.repo, limit),
      });
      await output(
        found
          ? {
              found: true,
              binary: found.binary,
              licenses: found.licenses,
              record: join(out, "host.json"),
              from: found.record.reusedFrom,
            }
          : { found: false },
      );
    } else if (command === "record") {
      if (!options.archive || !options.out)
        throw new Error("record needs --archive <zip> --out <json>");
      const reused = options.reused
        ? await readJson(options.reused)
        : undefined;
      const record = await hostRecord({
        target: target(),
        archive: resolve(options.archive),
        reused,
      });
      await mkdir(resolve(options.out, ".."), { recursive: true });
      await writeFile(
        resolve(options.out),
        `${JSON.stringify({ [targetInfo(target()).platform]: record }, null, 2)}\n`,
      );
      console.log(JSON.stringify(record, null, 2));
    } else
      throw new Error(
        "usage: host.mjs hash|versions|check-bump|fetch|record [--target <triple>] … (see the header)",
      );
  } catch (error) {
    // Every error ends here (fail() throws too): printed, exit code 1, and
    // never process.exit(), which right after hashing the host inputs can
    // deadlock Node 26 (the main thread waits for a V8 worker stuck in a
    // concurrent GC job); that hung main.yml for 20 minutes. Nothing is
    // left running here, so a normal exit with exitCode is immediate.
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}
