#!/usr/bin/env node
/**
 * deps.yml (Windows): the pinned upstream builds in deps.json "upstream",
 * verified and laid out the way setup-libmpv and bundle-windows.mjs use them:
 *
 *   <out>/
 *     libmpv-2.dll, include/mpv/*.h   our mpv-dev build (src/packaging/libmpv:
 *                                     MSYS2 UCRT64, FFmpeg 9.1 and the other
 *                                     libraries linked statically; a release
 *                                     asset pinned in deps.json)
 *     mpv.lib                         MSVC import library (deps.yml adds it)
 *     ffmpeg/ffmpeg.exe, ffprobe.exe, *.dll   BtbN win64 gpl-shared, carried
 *                                     over from the pinned deps release
 *     licenses/mpv/…, licenses/ffmpeg/…
 *     components.json
 *
 * The archive (.zip; shinchiro's were .7z) is unpacked with 7z when present,
 * else with tar (bsdtar reads both).
 *
 *   node packaging/collect-windows-deps.mjs --out <dir> [--cache <dir>]
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { zstdDecompressSync } from "node:zlib";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import {
  clean,
  copy,
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
  zipDirectory,
  zipRead,
} from "./lib.mjs";

reportErrors();
const options = parseArgs(process.argv.slice(2));
if (!options.out)
  fail("usage: collect-windows-deps.mjs --out <dir> [--cache <dir>]");
const out = resolve(options.out);
const cache = resolve(options.cache ?? join(tmpdir(), "vestra-deps-cache"));
const pins = await readJson(join(packaging, "deps.json"));
const { upstream } = pins;

/**
 * A release asset of a GitHub repository: through `gh` (its login, or
 * GH_TOKEN in CI), because this repository's releases are private and a
 * plain download of them is a 404; other URLs plainly.
 */
async function fetchPinned(pin) {
  const path = join(cache, basename(new URL(pin.url).pathname));
  if (!exists(path) || (await sha256File(path)) !== pin.sha256) {
    const url = new URL(pin.url);
    const [owner, repo, kind, verb, tag, file] = url.pathname
      .split("/")
      .slice(1);
    if (
      url.host === "github.com" &&
      `${owner}/${repo}` === "vestrasys/vestra-player" &&
      kind === "releases" &&
      verb === "download"
    ) {
      await mkdir(cache, { recursive: true });
      run("gh", [
        "release",
        "download",
        tag,
        "--repo",
        `${owner}/${repo}`,
        "--pattern",
        decodeURIComponent(file),
        "--dir",
        cache,
        "--clobber",
      ]);
    } else await download(pin.url, path);
  }
  await verifySha256(path, pin.sha256);
  return path;
}

await clean(out);
const mpv = upstream["windows-mpv"];

// mpv-dev: libmpv-2.dll and the headers.
const mpvArchive = await fetchPinned(mpv);
const unpacked = join(cache, "mpv-dev");
await clean(unpacked);
const has7z = !spawnSync("7z", ["i"], { stdio: "ignore" }).error;
if (has7z)
  run("7z", ["x", "-y", `-o${unpacked}`, mpvArchive], {
    stdio: ["ignore", "ignore", "inherit"],
  });
else run("tar", ["-xf", mpvArchive, "-C", unpacked]);
// The DLL itself is pinned too: the one that was tested (README "Shipping it").
if (mpv.libmpv_sha256)
  await verifySha256(join(unpacked, "libmpv-2.dll"), mpv.libmpv_sha256);
await copy(join(unpacked, "libmpv-2.dll"), join(out, "libmpv-2.dll"));
await copy(join(unpacked, "include/mpv"), join(out, "include/mpv"));
for (const file of ["Copyright", "LICENSE.GPL", "LICENSE.LGPL"])
  await download(
    `https://raw.githubusercontent.com/mpv-player/mpv/${mpv.mpv_commit}/${file}`,
    join(out, "licenses/mpv", file),
  );

// The Vulkan loader: libmpv-2.dll imports vulkan-1.dll, which Windows only
// has when a GPU driver installed it (not on CI runners or some VMs).
const vulkan = upstream["windows-vulkan-loader"];
const vulkanArchive = await fetchPinned(vulkan);
const vulkanFiles = untar(zstdDecompressSync(await readFile(vulkanArchive)));
for (const [from, to] of [
  ["ucrt64/bin/vulkan-1.dll", "vulkan-1.dll"],
  [
    "ucrt64/share/licenses/vulkan-loader/LICENSE",
    "licenses/vulkan-loader/LICENSE",
  ],
]) {
  if (!vulkanFiles.has(from)) fail(`${vulkanArchive} has no ${from}`);
  await mkdir(dirname(join(out, to)), { recursive: true });
  await writeFile(join(out, to), vulkanFiles.get(from));
}

// FFmpeg: the command-line tools and their shared libraries, the same bytes
// as in the deps release that deps.json pins now. BtbN deletes its
// autobuilds after a few weeks, so upstream["windows-ffmpeg"] records where
// they came from but can no longer be downloaded; the pinned release's zip
// (its sha256 is in deps.json) carries them on from release to release. A
// new FFmpeg CLI needs a new BtbN pin and a change here.
const previous = pins.assets["windows-x64"];
const previousZip = await fetchPinned({
  url: `https://github.com/vestrasys/vestra-player/releases/download/${pins.release}/${previous.file}`,
  sha256: previous.sha256,
});
const wanted = (await zipDirectory(previousZip)).entries
  .map((entry) => entry.name)
  .filter(
    (name) =>
      /^ffmpeg\/[^/]+\.(exe|dll)$/.test(name) ||
      name === "licenses/ffmpeg/LICENSE.txt",
  );
for (const name of ["ffmpeg/ffprobe.exe", "licenses/ffmpeg/LICENSE.txt"])
  if (!wanted.includes(name)) fail(`${previousZip} has no ${name}`);
for (const name of wanted) {
  const bytes = await zipRead(previousZip, name);
  if (!bytes) fail(`${previousZip}: cannot read ${name}`);
  await mkdir(dirname(join(out, name)), { recursive: true });
  await writeFile(join(out, name), bytes);
}
const ffmpegComponent = JSON.parse(
  (await zipRead(previousZip, "components.json")).toString("utf8"),
).components.find((component) => component.name === "ffmpeg");
if (!ffmpegComponent) fail(`${previousZip}: components.json has no ffmpeg`);

/** Regular files of a (ustar/pax) tar archive: path → bytes. */
function untar(bytes) {
  const files = new Map();
  let longName = null;
  for (let offset = 0; offset + 512 <= bytes.length;) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const field = (start, length) =>
      header.toString("utf8", start, start + length).replace(/\0.*$/s, "");
    const size = parseInt(field(124, 12).trim() || "0", 8);
    const type = field(156, 1) || "0";
    const prefix = field(345, 155);
    let name =
      longName ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100));
    longName = null;
    const data = bytes.subarray(offset + 512, offset + 512 + size);
    if (type === "x") {
      const path = /\d+ path=([^\n]*)\n/.exec(data.toString("utf8"));
      if (path) longName = path[1];
    } else if (type === "L")
      longName = data.toString("utf8").replace(/\0.*$/s, "");
    else if (type === "0") files.set(name.replace(/^\.\//, ""), data);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

const manifest = {
  schema: 1,
  platform: "windows-x64",
  generated: new Date().toISOString(),
  components: [
    {
      name: "vulkan-loader",
      version: vulkan.version,
      license: "Apache-2.0",
      homepage: "https://github.com/KhronosGroup/Vulkan-Loader",
      source: {
        url: `https://github.com/KhronosGroup/Vulkan-Loader/tree/v${vulkan.version}`,
        build: vulkan.build,
        archive: vulkan.url,
        archive_sha256: vulkan.sha256,
      },
      note: "vulkan-1.dll from the MSYS2 UCRT64 package; libmpv-2.dll imports it.",
      files: ["vulkan-1.dll"],
      license_files: ["licenses/vulkan-loader/LICENSE"],
    },
    {
      name: "mpv",
      version: `git-${mpv.mpv_commit.slice(0, 10)}`,
      license: "GPL-3.0-or-later",
      homepage: "https://mpv.io",
      source: {
        url: `https://github.com/mpv-player/mpv/tree/${mpv.mpv_commit}`,
        build: mpv.build,
        archive: mpv.url,
        archive_sha256: mpv.sha256,
      },
      note: `libmpv-2.dll built by src/packaging/libmpv/build-windows.sh in MSYS2 UCRT64: FFmpeg ${mpv.ffmpeg ?? "9.1"} (--enable-gpl --enable-version3) and mpv with mpv-lavfi-hwdevice.patch, with the MSYS2 libraries pinned in msys2-packages.lock (libass, freetype, fribidi, harfbuzz, libplacebo, shaderc, spirv-cross, luajit, mujs, uchardet, lcms2, libjpeg, zimg, libbluray, libdvdnav, libarchive, rubberband, OpenSSL, x264, x265, dav1d, libaribcaption, …) linked in statically; the combination is distributed under GPL-3.0-or-later. The patch and the scripts are in this repository.`,
      files: ["libmpv-2.dll"],
      license_files: [
        "licenses/mpv/Copyright",
        "licenses/mpv/LICENSE.GPL",
        "licenses/mpv/LICENSE.LGPL",
      ],
    },
    // Unchanged from the previous release, like its files.
    ffmpegComponent,
  ],
};
await writeFile(
  join(out, "components.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
console.log(`Windows dependencies in ${out}`);
