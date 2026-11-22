#!/bin/bash
# libmpv-2.dll for Windows x64 with FFmpeg's NVIDIA frame-rate converter
# (`fruc_vulkan`, FFmpeg 9.1+) and the lavfi `hwdevice=` patch
# (mpv-lavfi-hwdevice.patch), for motion smoothing's NVIDIA engine
# (src/host/src/playback/motion.rs).
#
# Why a source build: shinchiro's mpv-winbuild-cmake (deps.json
# "windows-mpv", the libmpv we ship) builds FFmpeg without a GLSL compiler,
# so FFmpeg leaves out every Vulkan filter that needs build-time SPIR-V,
# fruc_vulkan among them; and mpv's lavfi filter only gets a hardware device
# from the VO's interop, which Vestra's Direct3D 11 render API cannot give
# as Vulkan. The patch adds `hwdevice=<type>` / `hwdevice_name=<device>`:
# the filter creates its own device of that type when it is created (vf
# add), sets it on its graph's filters before they initialise, keeps it
# across graph rebuilds and frees it with the filter (nothing at playback
# start, no Vulkan device unless such a filter is in the chain).
#
# The configuration follows shinchiro's 20261021 recipes
# (packages/ffmpeg.cmake, packages/mpv.cmake: the same mpv commit, the same
# meson options, the same FFmpeg --enable list) with these differences.
# Each is this build environment's, not an upstream removal, and is left
# out only where neither the player nor its plugins use it:
#   FFmpeg: +glslc (spirv_compiler: the Vulkan filters, fruc_vulkan);
#           -avisynth (no headers in MSYS2; mpv never opens .avs);
#           -cuda-llvm (needs clang as the CUDA compiler: FFmpeg's CUDA
#            filters, which mpv's chain never uses; nvdec/cuvid stay);
#           -uavs3d, -davs2 (optional in shinchiro's too);
#           -opengl (the option is gone in FFmpeg 9.1: upstream);
#           libaribcaption built here (build_aribcaption), as shinchiro
#            does, with its bundled MD5 renamed: MSYS2's static library
#            clashes with OpenSSL's MD5_* in one static link;
#   mpv:    -subrandr (mpv's renderer for YouTube's srv3 captions, a Rust
#            library MSYS2 does not package; nothing in the player or its
#            plugins loads srv3, and WebVTT goes through the subtitles
#            plugin's own conversion), -pdf-build (docs only);
#           no clang thin LTO (gcc): sections and --gc-sections instead.
# Everything is linked statically into libmpv-2.dll as before, except the
# Vulkan loader (vulkan-1.dll, imported as before) and the Windows DLLs.
#
# Run in an MSYS2 UCRT64 shell (https://www.msys2.org, mpv's own Windows
# CI uses it), with the packages in msys2-packages.txt installed:
#   pacman -S --needed patch $(sed 's/^/mingw-w64-ucrt-x86_64-/' msys2-packages.txt)
#   ./build-windows.sh <work dir> [aribcaption|ffmpeg|mpv|package|all]
# Sources (pinned, checked by SHA-256):
FFMPEG_VERSION=9.1
FFMPEG_URL=https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz
FFMPEG_SHA256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433
MPV_COMMIT=e471496c56f0f49c133bd88b50aed62b845f1909
MPV_URL=https://github.com/mpv-player/mpv/archive/$MPV_COMMIT.tar.gz
MPV_SHA256=562674ce7a0f6ac0681291037abc4c8595e0d1202d86d27eb341113eb7d984ba
# Built here rather than taken from MSYS2 (as shinchiro's build does):
ARIBCC_VERSION=1.1.2
ARIBCC_URL=https://github.com/xqq/libaribcaption/archive/refs/tags/v$ARIBCC_VERSION.tar.gz
ARIBCC_SHA256=649b50bde99272b97c66af2a8400163e2f84eae072d252daa26baaaf0866a1c2
# Dead code out of the static link (gcc has no thin LTO across this many
# static libraries in reasonable time; sections and --gc-sections instead).
SECTIONS='-ffunction-sections -fdata-sections'
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
WORK=${1:?work directory}
STEP=${2:-all}
JOBS=${JOBS:-$(nproc)}
mkdir -p "$WORK"/{dl,src,prefix,out}
WORK=$(cd "$WORK" && pwd)
PREFIX=$WORK/prefix
export PKG_CONFIG_PATH=$PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}

fetch() { # url sha256 file
  [ -f "$3" ] || curl -fsSL -o "$3" "$1"
  echo "$2  $3" | sha256sum -c -
}

setup_pkgconfig() {
  # FFmpeg (libplacebo) and mpv ask pkg-config for shaderc and
  # spirv-cross-c-shared, which MSYS2
  # links as DLLs: the static libraries under those names instead (as
  # shinchiro's build), and the Vulkan loader's import library under a
  # name -static accepts (vulkan-1.dll stays imported, as before).
  mkdir -p "$PREFIX/lib/pkgconfig"
  # (pkgconf redefines `prefix` from where a .pc file lies: the paths are
  # written out; MSYS2's static shaderc and SPIRV-Cross C API need
  # glslang, SPIRV-Tools and SPIRV-Cross's other parts after them.)
  local m; m=$(cygpath -m "$MINGW_PREFIX")
  sed -e '/^prefix=/d' -e "s|\${prefix}|$m|g" -e 's/^Name: .*/Name: shaderc/' \
    -e 's/-lshaderc_combined$/-lshaderc_combined -lglslang -lMachineIndependent -lGenericCodeGen -lOSDependent -lglslang-default-resource-limits -lSPIRV -lSPIRV-Tools-opt -lSPIRV-Tools -lstdc++/' \
    "$MINGW_PREFIX/lib/pkgconfig/shaderc_combined.pc" > "$PREFIX/lib/pkgconfig/shaderc.pc"
  sed -e '/^prefix=/d' -e "s|\${prefix}|$m|g" -e 's/^Name: .*/Name: spirv-cross-c-shared/' \
    -e 's/-lspirv-cross-c$/-lspirv-cross-c -lspirv-cross-msl -lspirv-cross-hlsl -lspirv-cross-glsl -lspirv-cross-cpp -lspirv-cross-reflect -lspirv-cross-util -lspirv-cross-core -lstdc++/' \
    "$MINGW_PREFIX/lib/pkgconfig/spirv-cross-c.pc" > "$PREFIX/lib/pkgconfig/spirv-cross-c-shared.pc"
  cp "$MINGW_PREFIX/lib/libvulkan-1.dll.a" "$PREFIX/lib/libvulkan-1.a"
  # x265 and SRT name gcc's shared runtime (-lgcc_s, libgcc_s_seh-1.dll);
  # its static libgcc and libgcc_eh do instead.
  for pc in x265 srt haisrt; do
    sed -e '/^prefix=/d' -e "s|\${prefix}|$m|g" -e 's/ -lgcc_s\b//g' \
      "$MINGW_PREFIX/lib/pkgconfig/$pc.pc" > "$PREFIX/lib/pkgconfig/$pc.pc"
  done
}

# libaribcaption (FFmpeg's ARIB STD-B24 caption decoder) as shinchiro builds
# it: static, no RTTI, FreeType and Fontconfig, DirectWrite. Its bundled
# MD5 has OpenSSL's names (MD5_Init, MD5_Update, MD5_Final), which clash in
# one static link with OpenSSL's libcrypto: renamed with the preprocessor
# (md5.h and md5.c both see the macros, so its own calls follow).
build_aribcaption() {
  setup_pkgconfig
  fetch "$ARIBCC_URL" "$ARIBCC_SHA256" "$WORK/dl/libaribcaption-$ARIBCC_VERSION.tar.gz"
  local src=$WORK/src/libaribcaption-$ARIBCC_VERSION
  [ -d "$src" ] || tar -xzf "$WORK/dl/libaribcaption-$ARIBCC_VERSION.tar.gz" -C "$WORK/src"
  local rename="-DMD5_Init=aribcc_MD5_Init -DMD5_Update=aribcc_MD5_Update -DMD5_Final=aribcc_MD5_Final"
  cmake -S "$src" -B "$WORK/build-aribcaption" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DBUILD_SHARED_LIBS=OFF -DARIBCC_SHARED_LIBRARY=OFF -DARIBCC_BUILD_TESTS=OFF \
    -DARIBCC_USE_EMBEDDED_FREETYPE=OFF -DARIBCC_NO_RTTI=ON \
    -DARIBCC_USE_FONTCONFIG=ON -DARIBCC_USE_FREETYPE=ON \
    -DCMAKE_C_FLAGS="$rename $SECTIONS" -DCMAKE_CXX_FLAGS="$rename $SECTIONS"
  cmake --build "$WORK/build-aribcaption" -j "$JOBS"
  cmake --install "$WORK/build-aribcaption"
}

build_ffmpeg() {
  setup_pkgconfig
  fetch "$FFMPEG_URL" "$FFMPEG_SHA256" "$WORK/dl/ffmpeg-$FFMPEG_VERSION.tar.xz"
  local src=$WORK/src/ffmpeg-$FFMPEG_VERSION
  [ -d "$src" ] || tar -xf "$WORK/dl/ffmpeg-$FFMPEG_VERSION.tar.xz" -C "$WORK/src"
  mkdir -p "$WORK/build-ffmpeg" && cd "$WORK/build-ffmpeg"
  [ -f ffbuild/config.mak ] || "$src/configure" \
    --prefix="$PREFIX" \
    --arch=x86_64 --target-os=mingw32 \
    --pkg-config-flags=--static \
    --disable-shared --enable-static \
    --enable-runtime-cpudetect \
    --enable-gpl --enable-version3 \
    --enable-vapoursynth \
    --enable-libass --enable-libbluray --enable-libdvdnav --enable-libdvdread \
    --enable-libfreetype --enable-libfribidi --enable-libfontconfig --enable-libharfbuzz \
    --enable-libmodplug --enable-libopenmpt --enable-libmp3lame --enable-lcms2 \
    --enable-libopus --enable-libsoxr --enable-libspeex --enable-libvorbis --enable-libbs2b \
    --enable-librubberband --enable-libvpx --enable-libwebp --enable-libx264 --enable-libx265 \
    --enable-libaom --enable-libsvtav1 --enable-libdav1d --enable-libzimg \
    --enable-openssl --enable-libxml2 --enable-libmysofa --enable-libssh --enable-libsrt \
    --enable-libvpl --enable-libjxl --enable-libplacebo --enable-libzvbi --enable-libaribcaption \
    --enable-cuvid --enable-nvdec --enable-nvenc --enable-amf --enable-openal \
    --enable-vaapi --enable-vulkan --glslc=glslc \
    --disable-doc --disable-ffplay --disable-ffprobe --disable-ffmpeg \
    --disable-vdpau --disable-videotoolbox --disable-decoder=libaom_av1 \
    --extra-cflags="-Wno-error=int-conversion $SECTIONS" \
    --extra-libs='-lstdc++'
  # The acceptance check: the converter is in.
  grep -q '^CONFIG_FRUC_VULKAN_FILTER=yes' ffbuild/config.mak || { echo "fruc_vulkan was not enabled"; exit 1; }
  make -j"$JOBS"
  make install
}

build_mpv() {
  setup_pkgconfig
  fetch "$MPV_URL" "$MPV_SHA256" "$WORK/dl/mpv-$MPV_COMMIT.tar.gz"
  local src=$WORK/src/mpv-$MPV_COMMIT
  if [ ! -d "$src" ]; then
    tar -xzf "$WORK/dl/mpv-$MPV_COMMIT.tar.gz" -C "$WORK/src"
    patch -d "$src" -p1 < "$HERE/mpv-lavfi-hwdevice.patch"
  fi
  local build=$WORK/build-mpv
  # Static libraries built against DLLs (static-imp-shims.txt): each
  # __imp_<symbol> points at the static <symbol>. The references are weak,
  # so meson's link tests (without those libraries) still link, and
  # -u <symbol> pulls the static definitions into the real link. libsoxr's
  # OpenMP comes from gcc's static libgomp.
  local shims=$WORK/static-imp-shims
  grep -v '^#' "$HERE/static-imp-shims.txt" | while read -r s; do
    echo "extern void shim_$s(void) __asm__(\"$s\") __attribute__((weak));"
    echo "void *__imp_$s = (void *)shim_$s;"
  done > "$shims.c"
  gcc -c -O2 -o "$shims.o" "$shims.c"
  # OpenAL's static library defines the KSDATAFORMAT_SUBTYPE_* GUIDs that
  # mpv's WASAPI output defines too (INITGUID): the same constants, so the
  # first definition is kept.
  local link="-s -Wl,--no-insert-timestamp -Wl,--gc-sections -static -L$(cygpath -m "$PREFIX/lib") $(cygpath -m "$shims.o") -l:libgomp.a -Wl,--allow-multiple-definition"
  link+=$(grep -v '^#' "$HERE/static-imp-shims.txt" | sed 's/^/ -Wl,-u,/' | tr -d '\n')
  [ -f "$build/build.ninja" ] || meson setup "$build" "$src" \
    --prefix="$PREFIX" --libdir="$PREFIX/lib" \
    --default-library=shared --prefer-static \
    -Ddebug=true -Db_ndebug=true -Doptimization=3 \
    -Dlibmpv=true -Dcplayer=false \
    -Dpdf-build=disabled \
    -Dlua=enabled -Djavascript=enabled -Dsdl2-gamepad=enabled -Dlibarchive=enabled \
    -Dlibbluray=enabled -Ddvdnav=enabled -Duchardet=enabled -Drubberband=enabled \
    -Dlcms2=enabled -Dopenal=enabled -Dspirv-cross=enabled -Dvulkan=enabled \
    -Dvapoursynth=enabled -Dsubrandr=disabled -Dsixel=enabled \
    -Dwin32-smtc=enabled -Dcaca=disabled \
    -Dgl=enabled -Degl-angle=enabled -Dlibcurl=enabled \
    -Dc_args="-Wno-error=int-conversion $SECTIONS" -Dcpp_args="$SECTIONS" \
    -Dc_link_args="$link" \
    -Dcpp_link_args="$link"
  ninja -C "$build" -j"$JOBS"
}

package() {
  local build=$WORK/build-mpv out=$WORK/out
  # Stripped and without a link timestamp at link time (-s,
  # --no-insert-timestamp), so nothing rewrites the bytes here.
  cp "$build/libmpv-2.dll" "$out/"
  mkdir -p "$out/include/mpv"
  cp "$WORK/src/mpv-$MPV_COMMIT"/include/mpv/{client.h,render.h,render_gl.h,stream_cb.h} "$out/include/mpv/"
  # Every DLL it imports must be a Windows one or vulkan-1.dll.
  objdump -p "$out/libmpv-2.dll" | sed -n 's/^\s*DLL Name: //p' | sort -u > "$out/imports.txt"
  if grep -v -i -x -E '(vulkan-1|avicap32|bcryptprimitives|ntdll|shcore|usp10|wsock32|d2d1|kernel32|user32|gdi32|advapi32|shell32|ole32|oleaut32|ws2_32|bcrypt|crypt32|secur32|ncrypt|winmm|avrt|dwmapi|uxtheme|imm32|version|setupapi|cfgmgr32|shlwapi|comdlg32|mfplat|mfuuid|ksuser|iphlpapi|dnsapi|wldap32|normaliz|dbghelp|psapi|userenv|rpcrt4|hid|dinput8|dxgi|d3d11|d3d9|d3d12|opengl32|dwrite|dxva2|api-ms-win-[a-z0-9-]+|msvcrt|ucrtbase|winhttp|propsys|powrprof|wintrust|runtimeobject|combase)\.dll' "$out/imports.txt"; then
    echo "libmpv-2.dll imports DLLs beyond Windows' and vulkan-1.dll (above)"; exit 1
  fi
  (cd "$out" && sha256sum libmpv-2.dll > libmpv-2.dll.sha256)
  cat "$out/libmpv-2.dll.sha256"
  # The release asset deps.json's windows-mpv pins (the layout of an mpv-dev
  # archive: libmpv-2.dll and include/mpv at the top), with what it was
  # built from.
  {
    echo "libmpv-2.dll $(cut -d' ' -f1 "$out/libmpv-2.dll.sha256")"
    echo "ffmpeg $FFMPEG_VERSION $FFMPEG_SHA256"
    echo "mpv $MPV_COMMIT $MPV_SHA256"
    echo "libaribcaption $ARIBCC_VERSION $ARIBCC_SHA256"
    echo "mpv-lavfi-hwdevice.patch $(sha256sum "$HERE/mpv-lavfi-hwdevice.patch" | cut -d' ' -f1)"
    echo "build-windows.sh $(sha256sum "$HERE/build-windows.sh" | cut -d' ' -f1)"
    echo "msys2-packages.lock $(sha256sum "$HERE/msys2-packages.lock" | cut -d' ' -f1)"
  } > "$out/BUILD-INFO.txt"
  cp "$HERE/mpv-lavfi-hwdevice.patch" "$out/"
  local asset=$WORK/libmpv-dev-x86_64-ffmpeg$FFMPEG_VERSION-mpv-${MPV_COMMIT:0:10}.zip
  rm -f "$asset"
  (cd "$out" && bsdtar -a -cf "$asset" libmpv-2.dll include BUILD-INFO.txt mpv-lavfi-hwdevice.patch)
  echo "$(sha256sum "$asset" | cut -d' ' -f1)  $(basename "$asset")"
}

case $STEP in
  aribcaption) build_aribcaption ;;
  ffmpeg) build_ffmpeg ;;
  mpv) build_mpv ;;
  package) package ;;
  all)
    "$HERE/setup-msys2.sh" check || { echo "MSYS2 differs from msys2-packages.lock: run ./setup-msys2.sh install"; exit 1; }
    build_aribcaption; build_ffmpeg; build_mpv; package ;;
  *) echo "step: aribcaption | ffmpeg | mpv | package | all"; exit 2 ;;
esac
