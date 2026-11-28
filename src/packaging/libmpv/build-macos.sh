#!/bin/bash
# libmpv.2.dylib for macOS arm64 with Apple's frame interpolation as an mpv
# video filter (`vtfrc`, mpv-vt-frc.patch), for motion smoothing's Apple
# engine (src/host/src/playback/motion.rs), and the lavfi `hwdevice=` patch
# the Windows build carries (mpv-lavfi-hwdevice.patch), so both platforms
# run the same mpv.
#
# Why a source build: Homebrew's mpv is a release (0.41.0) without either
# patch. This builds mpv at the commit of the Windows build (MPV_COMMIT,
# build-windows.sh) against the installed Homebrew FFmpeg and libraries:
# FFmpeg is not rebuilt. The meson options are Homebrew's mpv formula's
# (`brew cat mpv`: lua=luajit, javascript, libarchive, uchardet, vulkan,
# no build date) without its docs. The CLI is built from the same tree, for
# measurements only (not shipped).
#
# Requirements: Xcode command line tools (clang, swiftc, the macOS SDK),
# Homebrew with mpv's dependencies installed (`brew deps mpv`; Homebrew's mpv
# itself is not used), python3 (for meson). meson and ninja are installed
# into <work dir>/venv from the wheels pinned below; nothing is installed into
# the system.
#
#   ./build-macos.sh <work dir> [tools|mpv|package|all]
#
#   tools    venv with meson and ninja (pinned wheels, SHA-256 checked)
#   mpv      fetch (pinned), patch, configure and build; installs into
#            <work dir>/stage/usr/local (the prefix the binaries name is the
#            neutral /usr/local, not the work directory): lib/libmpv.2.dylib,
#            include/mpv, bin/mpv
#   package  the relocatable deps directory <work dir>/deps: the dylib
#            closure of this libmpv, Homebrew's ffmpeg and ffprobe
#            (collect-macos-deps.mjs --libmpv <work dir>/stage/usr/local)
#   all      the three in order
#
# Sources (pinned, checked by SHA-256): mpv as in build-windows.sh.
MPV_COMMIT=e471496c56f0f49c133bd88b50aed62b845f1909
MPV_URL=https://github.com/mpv-player/mpv/archive/$MPV_COMMIT.tar.gz
MPV_SHA256=562674ce7a0f6ac0681291037abc4c8595e0d1202d86d27eb341113eb7d984ba
MESON_WHEEL=meson-1.12.1-py3-none-any.whl
MESON_URL=https://files.pythonhosted.org/packages/py3/m/meson/$MESON_WHEEL
MESON_SHA256=930bc7542cbd9f57009e182fd014eba48cf1a0180a7b9006d2d32d8f168d8b02
NINJA_WHEEL=ninja-1.13.2-py3-none-macosx_10_9_universal2.whl
NINJA_URL=https://files.pythonhosted.org/packages/py3/n/ninja/$NINJA_WHEEL
NINJA_SHA256=fd82e26c0706ad4ab88e5fdd26f3fab0a987a90f810160f6c322e752c6af298b
PATCHES=(mpv-lavfi-hwdevice.patch mpv-vt-frc.patch)
# The oldest macOS the build runs on (the player's minimum, doc/process/ci.md);
# the filter checks for macOS 15.4 (VTFrameRateConversion) and 26
# (VTLowLatencyFrameInterpolation) at run time. The Homebrew libraries it
# links may need a newer one: collect-macos-deps.mjs records the highest
# (components.json min_macos).
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-15.0}
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
WORK=${1:?work directory}
STEP=${2:-all}
JOBS=${JOBS:-$(sysctl -n hw.perflevel0.physicalcpu 2>/dev/null || sysctl -n hw.ncpu)}
mkdir -p "$WORK"/{dl,src}
WORK=$(cd "$WORK" && pwd)
# Installed with --destdir: the binaries name /usr/local (mpv's config
# directory, its embedded configure line), not the work directory.
INSTALL_PREFIX=/usr/local
PREFIX=$WORK/stage$INSTALL_PREFIX
BREW=$(brew --prefix)
# libarchive is keg-only in Homebrew (as in its mpv formula).
export PKG_CONFIG_PATH=$BREW/opt/libarchive/lib/pkgconfig:$BREW/lib/pkgconfig
export PATH=$WORK/venv/bin:$PATH

fetch() { # url sha256 file
  [ -f "$3" ] || curl -fsSL -o "$3" "$1"
  echo "$2  $3" | shasum -a 256 -c -
}

tools() {
  fetch "$MESON_URL" "$MESON_SHA256" "$WORK/dl/$MESON_WHEEL"
  fetch "$NINJA_URL" "$NINJA_SHA256" "$WORK/dl/$NINJA_WHEEL"
  [ -x "$WORK/venv/bin/python" ] || python3 -m venv "$WORK/venv"
  "$WORK/venv/bin/python" -m pip install -q --no-index --no-deps \
    "$WORK/dl/$MESON_WHEEL" "$WORK/dl/$NINJA_WHEEL"
  meson --version && ninja --version
}

build_mpv() {
  command -v meson > /dev/null || tools
  fetch "$MPV_URL" "$MPV_SHA256" "$WORK/dl/mpv-$MPV_COMMIT.tar.gz"
  local src=$WORK/src/mpv-$MPV_COMMIT
  if [ ! -d "$src" ]; then
    tar -xzf "$WORK/dl/mpv-$MPV_COMMIT.tar.gz" -C "$WORK/src"
    for patch in "${PATCHES[@]}"; do
      patch -d "$src" -p1 < "$HERE/$patch"
    done
  fi
  local build=$WORK/build-mpv
  # The work directory's path stays out of the binaries: __FILE__ in
  # assertions and log lines is mapped (given through the environment, so
  # mpv's embedded configure line does not carry the path either).
  local map="-ffile-prefix-map=$src=mpv -fmacro-prefix-map=$src=mpv"
  [ -f "$build/build.ninja" ] || CFLAGS="$map" OBJCFLAGS="$map" meson setup "$build" "$src" \
    --prefix="$INSTALL_PREFIX" --buildtype=release \
    -Db_ndebug=true \
    -Dlibmpv=true -Dcplayer=true -Dbuild-date=false \
    -Dhtml-build=disabled -Dmanpage-build=disabled -Dpdf-build=disabled \
    -Dlua=luajit -Djavascript=enabled -Dlibarchive=enabled \
    -Duchardet=enabled -Dvulkan=enabled \
    -Dvideotoolbox-frc=enabled
  ninja -C "$build" -j"$JOBS"
  meson install -C "$build" --quiet --no-rebuild --destdir "$WORK/stage"
  # The acceptance check: the filter is in, both patches applied.
  # (Output captured first: grep -q closing the pipe would fail mpv.)
  local help; help=$("$PREFIX/bin/mpv" --no-config --vf=help)
  grep -q '^  vtfrc ' <<< "$help" || { echo "vtfrc is not in the build"; exit 1; }
  help=$("$PREFIX/bin/mpv" --no-config --vf=lavfi=help 2>&1 || true)
  grep -q 'hwdevice' <<< "$help" || { echo "lavfi has no hwdevice option"; exit 1; }
  # What collect-macos-deps.mjs --libmpv needs to list this libmpv as a
  # component (components.json): version, license, source and patches.
  mkdir -p "$PREFIX/licenses" "$PREFIX/patches"
  cp "$src"/{LICENSE.GPL,LICENSE.LGPL,Copyright} "$PREFIX/licenses/"
  cp "${PATCHES[@]/#/$HERE/}" "$PREFIX/patches/"
  local version; version="$(sed 's/-UNKNOWN$//' "$src/MPV_VERSION")+${MPV_COMMIT:0:10}"
  {
    echo "{"
    echo "  \"name\": \"mpv\","
    echo "  \"version\": \"$version\","
    echo "  \"license\": \"GPL-2.0-or-later AND LGPL-2.1-or-later\","
    echo "  \"homepage\": \"https://mpv.io\","
    echo "  \"build\": \"src/packaging/libmpv/build-macos.sh\","
    echo "  \"source\": { \"url\": \"$MPV_URL\", \"sha256\": \"$MPV_SHA256\", \"revision\": \"$MPV_COMMIT\", \"archive_path\": \"$WORK/dl/mpv-$MPV_COMMIT.tar.gz\" },"
    echo "  \"patches\": ["
    local n=0
    for patch in "${PATCHES[@]}"; do
      [ $n -eq 0 ] || echo ","
      printf '    { "file": "%s", "sha256": "%s" }' "$patch" "$(shasum -a 256 "$HERE/$patch" | cut -d' ' -f1)"
      n=$((n + 1))
    done
    echo
    echo "  ]"
    echo "}"
  } > "$PREFIX/vestra-libmpv.json"
  {
    echo "mpv $MPV_COMMIT $MPV_SHA256"
    for patch in "${PATCHES[@]}"; do
      echo "$patch $(shasum -a 256 "$HERE/$patch" | cut -d' ' -f1)"
    done
    echo "build-macos.sh $(shasum -a 256 "$HERE/build-macos.sh" | cut -d' ' -f1)"
    echo "homebrew $(brew --version | head -1)"
    brew list --versions $(brew deps mpv) | sed 's/^/  /'
    echo "MACOSX_DEPLOYMENT_TARGET $MACOSX_DEPLOYMENT_TARGET"
    echo "libmpv.2.dylib $(shasum -a 256 "$PREFIX/lib/libmpv.2.dylib" | cut -d' ' -f1)"
  } > "$PREFIX/BUILD-INFO.txt"
  cat "$PREFIX/BUILD-INFO.txt" | head -4
}

package() {
  node "$HERE/../collect-macos-deps.mjs" --libmpv "$PREFIX" --out "$WORK/deps"
}

case $STEP in
  tools) tools ;;
  mpv) build_mpv ;;
  package) package ;;
  all) tools; build_mpv; package ;;
  *) echo "step: tools | mpv | package | all"; exit 2 ;;
esac
