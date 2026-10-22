#!/usr/bin/env node
/**
 * deps.yml (Windows): the pinned upstream builds in deps.json "upstream",
 * verified and laid out the way setup-libmpv and bundle-windows.mjs use them:
 *
 *   <out>/
 *     libmpv-2.dll, include/mpv/*.h   shinchiro mpv-dev (MinGW, FFmpeg and the
 *                                     other libraries linked statically)
 *     mpv.lib                         MSVC import library (deps.yml adds it)
 *     ffmpeg/ffmpeg.exe, ffprobe.exe, *.dll   BtbN win64 gpl-shared
 *     licenses/mpv/…, licenses/ffmpeg/…
 *     components.json
 *
 * The .7z is unpacked with 7z when present, else with tar (bsdtar reads 7z).
 *
 *   node packaging/collect-windows-deps.mjs --out <dir> [--cache <dir>]
 */
import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
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
  run,
  sha256File,
  verifySha256,
  zipDirectory,
  zipRead,
} from "./lib.mjs";

const options = parseArgs(process.argv.slice(2));
if (!options.out)
  fail("usage: collect-windows-deps.mjs --out <dir> [--cache <dir>]");
const out = resolve(options.out);
const cache = resolve(options.cache ?? join(tmpdir(), "vestra-deps-cache"));
const { upstream } = await readJson(join(packaging, "deps.json"));

async function fetchPinned(pin) {
  const path = join(cache, basename(new URL(pin.url).pathname));
  if (!exists(path) || (await sha256File(path)) !== pin.sha256)
    await download(pin.url, path);
  await verifySha256(path, pin.sha256);
  return path;
}

await clean(out);
const mpv = upstream["windows-mpv"];
const ffmpeg = upstream["windows-ffmpeg"];

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
await copy(join(unpacked, "libmpv-2.dll"), join(out, "libmpv-2.dll"));
await copy(join(unpacked, "include/mpv"), join(out, "include/mpv"));
for (const file of ["Copyright", "LICENSE.GPL", "LICENSE.LGPL"])
  await download(
    `https://raw.githubusercontent.com/mpv-player/mpv/${mpv.mpv_commit}/${file}`,
    join(out, "licenses/mpv", file),
  );

// FFmpeg: the command-line tools and their shared libraries.
const ffArchive = await fetchPinned(ffmpeg);
const { entries } = await zipDirectory(ffArchive);
const top = entries[0].name.split("/")[0];
const wanted = entries
  .map((entry) => entry.name)
  .filter(
    (name) =>
      /^[^/]+\/bin\/(ffmpeg\.exe|ffprobe\.exe|[^/]+\.dll)$/.test(name) ||
      name === `${top}/LICENSE.txt`,
  );
if (!wanted.some((name) => name.endsWith("/ffprobe.exe")))
  fail(`${ffArchive} has no ffprobe.exe`);
await mkdir(join(out, "ffmpeg"), { recursive: true });
await mkdir(join(out, "licenses/ffmpeg"), { recursive: true });
for (const name of wanted) {
  const target = name.endsWith("LICENSE.txt")
    ? join(out, "licenses/ffmpeg/LICENSE.txt")
    : join(out, "ffmpeg", basename(name));
  await writeFile(target, await zipRead(ffArchive, name));
}

const manifest = {
  schema: 1,
  platform: "windows-x64",
  generated: new Date().toISOString(),
  components: [
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
      note: "libmpv-2.dll from shinchiro/mpv-winbuild-cmake. FFmpeg (--enable-gpl --enable-version3) and the libraries in that build's packages/ (libass, freetype, fribidi, harfbuzz, libplacebo, shaderc, spirv-cross, vulkan, luajit, mujs, uchardet, lcms2, libjpeg, zimg, libbluray, libdvdnav, libarchive, rubberband, x264, x265, dav1d, …) are linked in statically; the combination is distributed under GPL-3.0-or-later.",
      files: ["libmpv-2.dll"],
      license_files: [
        "licenses/mpv/Copyright",
        "licenses/mpv/LICENSE.GPL",
        "licenses/mpv/LICENSE.LGPL",
      ],
    },
    {
      name: "ffmpeg",
      version: ffmpeg.version,
      license: "GPL-3.0-or-later",
      homepage: "https://ffmpeg.org",
      source: {
        url: `https://github.com/FFmpeg/FFmpeg/tree/${ffmpeg.version}`,
        build: ffmpeg.build,
        archive: ffmpeg.url,
        archive_sha256: ffmpeg.sha256,
      },
      note: "BtbN FFmpeg-Builds win64 gpl-shared: ffmpeg.exe, ffprobe.exe and the av*/sw* DLLs.",
      files: wanted
        .filter((name) => !name.endsWith("LICENSE.txt"))
        .map((name) => `ffmpeg/${basename(name)}`)
        .sort(),
      license_files: ["licenses/ffmpeg/LICENSE.txt"],
    },
  ],
};
await writeFile(
  join(out, "components.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
console.log(`Windows dependencies in ${out}`);
