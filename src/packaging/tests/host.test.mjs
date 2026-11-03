// The host input hash and the reuse of a released host (packaging/host.mjs).
import assert from "node:assert/strict";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { crc32 } from "node:zlib";
import {
  checkHostBump,
  fetchReleasedHost,
  HostError,
  hostInputs,
  hostRecord,
  RECIPE,
  resolveHost,
  TARGETS,
} from "../host.mjs";
import {
  compareVersions,
  hostVersion,
  packaging,
  parseVersion,
  sha256,
} from "../lib.mjs";

const WINDOWS = "x86_64-pc-windows-msvc";

/** A stored (uncompressed) zip of { name: Buffer }. */
function zip(files) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, data] of Object.entries(files)) {
    const path = Buffer.from(name);
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(path.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(path.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, path, data);
    centrals.push(central, path);
    offset += local.length + path.length + data.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

test("the host input hash is stable and covers what the binary is built from", async () => {
  const first = await hostInputs(WINDOWS);
  const second = await hostInputs(WINDOWS);
  assert.equal(first.hash, second.hash);
  assert.match(first.hash, /^[0-9a-f]{64}$/);
  const files = first.manifest
    .split("\n")
    .filter((line) => line.startsWith("file "))
    .map((line) => line.split(" ").slice(2).join(" "));
  for (const path of [
    "rust-toolchain.toml",
    "src/host/Cargo.toml",
    "src/host/Cargo.lock",
    "src/host/build.rs",
    "src/host/.cargo/config.toml",
    "src/packaging/about.toml",
    // include_bytes! from src/host/src/font_asset.rs
    "src/assets/fonts/noto-cjk/NotoSansCJKsc-Regular.otf",
  ])
    assert.ok(files.includes(path), `${path} is a host input`);
  for (const path of files) {
    assert.doesNotMatch(path, /^src\/host\/unsafe\/gpu-probe\//);
    assert.doesNotMatch(path, /^src\/host\/.*\.md$/);
    assert.doesNotMatch(path, /^src\/(plugins|runtime|scripts)\//);
    assert.notEqual(path, "src/release.json");
  }
  assert.match(
    first.manifest,
    /^deps vestra-deps-windows-x64\.zip [0-9a-f]{64}$/m,
  );
  // The target is an input: the two packaged hosts never share a hash.
  assert.notEqual(first.hash, (await hostInputs("aarch64-apple-darwin")).hash);
});

test("release ≥ host: semantic version order", () => {
  assert.ok(compareVersions("0.7.2", "0.7.1") > 0);
  assert.ok(compareVersions("0.10.0", "0.9.9") > 0);
  assert.equal(compareVersions("1.2.3", "1.2.3"), 0);
  assert.ok(compareVersions("1.2.3-rc.1", "1.2.3") < 0);
  assert.throws(() => compareVersions("1.2", "1.2.0"));
  // Build metadata does not count; pre-release identifiers per semver §11.
  assert.equal(compareVersions("1.2.3+a", "1.2.3+b"), 0);
  assert.ok(compareVersions("1.2.3-alpha.2", "1.2.3-alpha.10") < 0);
  assert.ok(compareVersions("1.0.0-alpha", "1.0.0-alpha.1") < 0);
  assert.ok(compareVersions("1.0.0-alpha.1", "1.0.0-alpha.beta") < 0);
  assert.ok(compareVersions("1.0.0-beta", "1.0.0-rc.1") < 0);
  assert.ok(compareVersions("1.0.0-rc.1+build.9", "1.0.0") < 0);
});

test("versions parse as the host's version::parse does", () => {
  for (const version of [
    "1.2.3",
    "0.0.0",
    "1.2.3-rc.1",
    "1.2.3+build.5",
    "1.2.3-rc.1+build.5",
    "1.2.3+b-c",
    "4294967295.0.0",
  ])
    assert.ok(parseVersion(version), version);
  assert.deepEqual(parseVersion("1.2.3-rc.1+b.2"), {
    core: [1, 2, 3],
    pre: "rc.1",
    build: "b.2",
  });
  assert.deepEqual(parseVersion("1.2.3+b-c"), {
    core: [1, 2, 3],
    pre: undefined,
    build: "b-c",
  });
  for (const version of [
    "1.2",
    "1.2.3.4",
    "01.2.3",
    "1.02.3",
    "v1.2.3",
    " 1.2.3",
    "4294967296.0.0",
    "1.2.3-é",
    `1.2.3-${"x".repeat(60)}`,
    "",
    undefined,
    7,
  ])
    assert.equal(parseVersion(version), undefined, String(version));
});

test("RECIPE names the cargo-about version the package workflow installs", async () => {
  const workflow = await readFile(
    join(packaging, "../../.github/workflows/_package.yml"),
    "utf8",
  );
  const installed = /tool: cargo-about@([0-9.]+)/.exec(workflow)?.[1];
  assert.ok(installed, "_package.yml installs cargo-about");
  assert.ok(
    RECIPE.endsWith(`cargo-about ${installed}`),
    `RECIPE (${RECIPE}) must name cargo-about ${installed}`,
  );
});

/** A fake published release v1.0.1 with a Windows host built from `hash`. */
async function fakeRelease(hash, { tamper, version } = {}) {
  const dir = await mkdtemp(join(tmpdir(), "vestra-host-release-"));
  const exe = Buffer.from("MZ fake host");
  const html = Buffer.from("<html>crates</html>");
  const top = "VestraPlayer-1.0.1-windows-x64";
  const archive = TARGETS[WINDOWS].archive("1.0.1");
  await writeFile(
    join(dir, archive),
    zip({
      [`${top}/vestra-host.exe`]: exe,
      [`${top}/licenses/rust-crates.html`]: html,
      [`${top}/resources/release.json`]: Buffer.from('{"version":"1.0.1"}'),
    }),
  );
  const record = await hostRecord({
    target: WINDOWS,
    archive: join(dir, archive),
    reused: {
      inputHash: (await hostInputs(WINDOWS)).hash,
      commit: "c0ffee",
      reusedFrom: null,
    },
  });
  record.inputHash = hash;
  if (version) record.hostVersion = version;
  if (tamper === "binary") record.binarySha256 = sha256("other");
  const host = JSON.stringify({
    schema: 1,
    release: "1.0.1",
    hosts: { "windows-x64": record },
  });
  await writeFile(join(dir, "host.json"), host);
  let sums = `${sha256(host)}  host.json\n${sha256(await readFile(join(dir, archive)))}  ${archive}\n`;
  if (tamper === "archive")
    sums = sums.replace(/^[0-9a-f]{64}(?=  V)/m, "0".repeat(64));
  await writeFile(join(dir, "SHA256SUMS"), sums);
  const from = {
    name: "fake",
    releases: async () => [
      { tag: "v1.0.2", assets: ["SHA256SUMS"] },
      { tag: "v1.0.1", assets: ["host.json", "SHA256SUMS", archive] },
    ],
    download: async (tag, names, out) => {
      assert.equal(tag, "v1.0.1");
      for (const name of names)
        await copyFile(join(dir, name), join(out, name));
    },
  };
  return { dir, from, exe, html, record };
}

test("a released host with the same input hash is reused, verified", async () => {
  const hash = "a".repeat(64);
  const release = await fakeRelease(hash);
  const out = join(release.dir, "out");
  await mkdir(out);
  try {
    const found = await fetchReleasedHost({
      target: WINDOWS,
      hash,
      out,
      from: release.from,
    });
    assert.ok(found);
    assert.deepEqual(await readFile(found.binary), release.exe);
    assert.deepEqual(await readFile(found.licenses), release.html);
    assert.equal(found.record.reusedFrom, "v1.0.1");
    assert.equal(found.record.commit, "c0ffee");
    assert.equal(
      found.record.binary,
      "VestraPlayer-1.0.1-windows-x64/vestra-host.exe",
    );
    // Another hash under a new host version: nothing to reuse, so the
    // caller builds.
    assert.equal(
      await fetchReleasedHost({
        target: WINDOWS,
        hash: "b".repeat(64),
        version: "99.0.0",
        out,
        from: release.from,
      }),
      undefined,
    );
  } finally {
    await rm(release.dir, { recursive: true, force: true });
  }
});

for (const tamper of ["binary", "archive"])
  test(`a released host whose ${tamper} does not check out is refused`, async () => {
    const hash = "a".repeat(64);
    const release = await fakeRelease(hash, { tamper });
    const out = join(release.dir, "out");
    await mkdir(out);
    try {
      await assert.rejects(
        fetchReleasedHost({ target: WINDOWS, hash, out, from: release.from }),
        /SHA-256/,
      );
    } finally {
      await rm(release.dir, { recursive: true, force: true });
    }
  });

test("a host version released with other inputs must be raised", async () => {
  const version = await hostVersion();
  const release = await fakeRelease("a".repeat(64), { version });
  const out = join(release.dir, "out");
  await mkdir(out);
  try {
    // The same host version, other inputs: two binaries, one version.
    await assert.rejects(
      fetchReleasedHost({
        target: WINDOWS,
        hash: "b".repeat(64),
        version,
        out,
        from: release.from,
      }),
      (error) =>
        error instanceof HostError &&
        error.kind === "bump" &&
        /raise version in src\/host\/Cargo\.toml/.test(error.message),
    );
    // A lower host version than one released.
    await assert.rejects(
      fetchReleasedHost({
        target: WINDOWS,
        hash: "b".repeat(64),
        version: "0.0.1",
        out,
        from: release.from,
      }),
      (error) => error instanceof HostError && error.kind === "bump",
    );
  } finally {
    await rm(release.dir, { recursive: true, force: true });
  }
});

test("checkHostBump: same inputs reuse, a new version passes, other platforms do not count", () => {
  const released = [
    {
      tag: "v1.0.2",
      hosts: {
        "windows-x64": {
          target: WINDOWS,
          hostVersion: "1.0.1",
          inputHash: "a",
        },
        "macos-arm64": {
          target: "aarch64-apple-darwin",
          hostVersion: "1.0.1",
          inputHash: "m",
        },
      },
    },
  ];
  const check = (version, hash) =>
    checkHostBump({ target: WINDOWS, hash, version, released });
  assert.equal(check("1.0.1", "a").tag, "v1.0.2");
  assert.equal(check("1.0.2", "b"), undefined);
  assert.throws(() => check("1.0.1", "b"), /already released in v1\.0\.2/);
  assert.throws(() => check("1.0.0", "b"), /lower than host 1\.0\.1/);
});

/** resolveHost with a fake release list, a scratch cache and no build. */
async function resolveWith(from) {
  const cache = await mkdtemp(join(tmpdir(), "vestra-host-cache-"));
  const saved = process.env.VESTRA_HOST_CACHE;
  process.env.VESTRA_HOST_CACHE = cache;
  try {
    return await resolveHost({ target: WINDOWS, from, build: false });
  } finally {
    if (saved === undefined) delete process.env.VESTRA_HOST_CACHE;
    else process.env.VESTRA_HOST_CACHE = saved;
    await rm(cache, { recursive: true, force: true });
  }
}

test("--reuse-host: no matching release builds, a bad release stops", async () => {
  const hash = (await hostInputs(WINDOWS)).hash;
  // No release with this hash (an older host version): falls through to the
  // build, which build: false turns into this error.
  const older = await fakeRelease("a".repeat(64), { version: "0.0.1" });
  // The matching release, but its archive does not check out.
  const tampered = await fakeRelease(hash, { tamper: "archive" });
  try {
    await assert.rejects(
      resolveWith(older.from),
      (error) =>
        !(error instanceof HostError) &&
        /no host built from input hash/.test(error.message),
    );
    await assert.rejects(
      resolveWith(tampered.from),
      (error) => error instanceof HostError && error.kind === "verification",
    );
    // The lookup itself unavailable (no gh login, offline): a warning, then
    // the build.
    await assert.rejects(
      resolveWith({
        name: "offline",
        releases: async () => {
          throw new Error("gh: not logged in");
        },
        download: async () => {},
      }),
      /no host built from input hash/,
    );
  } finally {
    await rm(older.dir, { recursive: true, force: true });
    await rm(tampered.dir, { recursive: true, force: true });
  }
});
