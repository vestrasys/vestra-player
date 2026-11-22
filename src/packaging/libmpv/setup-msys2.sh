#!/bin/bash
# The MSYS2 packages build-windows.sh builds with, pinned to exact versions
# (msys2-packages.lock: package, version, file, SHA-256).
#
#   ./setup-msys2.sh lock                 write the lock from this MSYS2's installed packages
#                                         (their files must still be in pacman's cache)
#   ./setup-msys2.sh install [source]     install exactly the locked files, checked by SHA-256;
#                                         source: a directory with the files (the archive kept
#                                         next to a libmpv release), else repo.msys2.org
#   ./setup-msys2.sh archive <dir>        copy the locked files from pacman's cache into <dir>
#                                         (to keep with a libmpv release: MSYS2 prunes old
#                                         package versions from its repository in time)
#   ./setup-msys2.sh check                fail when an installed version differs from the lock
#   ./setup-msys2.sh linked <work dir>    write msys2-linked.txt: the packages whose static
#                                         libraries are linked into libmpv-2.dll (from the
#                                         link line in <work dir>/build-mpv/build.ninja)
#   ./setup-msys2.sh sources <dir> [lock] the MSYS2 source packages (<base>-<version>.src.tar.zst,
#                                         repo.msys2.org/mingw/sources) of the packages in
#                                         msys2-linked.txt, into <dir>, checked against
#                                         msys2-sources.lock; `lock` writes that lock instead
#                                         (the GPL source of the libraries in the DLL; optional,
#                                         for when the source is published)
#
# Run in an MSYS2 UCRT64 shell.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
LOCK=$HERE/msys2-packages.lock
LINKED=$HERE/msys2-linked.txt
SOURCES_LOCK=$HERE/msys2-sources.lock
CACHE=/var/cache/pacman/pkg
REPO=https://repo.msys2.org

url_of() { # file name -> repository URL
  case $1 in
    mingw-w64-ucrt-x86_64-*) echo "$REPO/mingw/ucrt64/$1" ;;
    *) echo "$REPO/msys/x86_64/$1" ;;
  esac
}

case ${1:-} in
  lock)
    {
      echo "# package version file sha256 (setup-msys2.sh lock, $(date -u +%Y-%m-%d))"
      echo "# MSYS2 base: https://repo.msys2.org/distrib/x86_64/msys2-base-x86_64-20261120.sfx.exe (sha256 4f6dadfdb2c9be5fc1938e30b635cb72bb4344c35e8b6fd225bc3250fcab5897); its own packages have file \"-\""
      pacman -Q | while read -r name version; do
        file=$(ls "$CACHE/$name-$version-"*.pkg.tar.* 2>/dev/null | grep -v '\.sig$' | head -1 || true)
        # Packages of the MSYS2 base archive itself have no cached file: "-".
        if [ -z "$file" ]; then echo "$name $version - -"; continue; fi
        echo "$name $version $(basename "$file") $(sha256sum "$file" | cut -d' ' -f1)"
      done
    } > "$LOCK"
    echo "wrote $LOCK ($(grep -vc '^#' "$LOCK") packages)"
    ;;
  install)
    source=${2:-}
    tmp=$(mktemp -d)
    grep -v '^#' "$LOCK" | while read -r name version file sha; do
      if [ "$(pacman -Q "$name" 2>/dev/null | cut -d' ' -f2)" = "$version" ]; then continue; fi
      [ "$file" != "-" ] || { echo "$name $version comes with the MSYS2 base archive (see the lock's header)" >&2; exit 1; }
      if [ -n "$source" ]; then cp "$source/$file" "$tmp/"; else curl -fsSL -o "$tmp/$file" "$(url_of "$file")"; fi
      echo "$sha  $tmp/$file" | sha256sum -c - >/dev/null
      echo "$tmp/$file"
    done > "$tmp/list"
    if [ -s "$tmp/list" ]; then pacman -U --noconfirm --needed $(cat "$tmp/list"); fi
    rm -rf "$tmp"
    "$0" check
    ;;
  archive)
    dir=${2:?directory}
    mkdir -p "$dir"
    grep -v '^#' "$LOCK" | while read -r name version file sha; do
      [ "$file" != "-" ] || continue
      cp "$CACHE/$file" "$dir/"
      echo "$sha  $dir/$file" | sha256sum -c - >/dev/null
    done
    echo "archived $(grep -vc '^#' "$LOCK") packages in $dir"
    ;;
  check)
    bad=0
    while read -r name version file sha; do
      have=$(pacman -Q "$name" 2>/dev/null | cut -d' ' -f2 || true)
      [ "$have" = "$version" ] || { echo "$name: installed ${have:-none}, locked $version"; bad=1; }
    done < <(grep -v '^#' "$LOCK")
    [ $bad = 0 ] && echo "MSYS2 packages match the lock"
    exit $bad
    ;;
  linked)
    work=$(cd "${2:?work directory}" && pwd)
    ninja=$work/build-mpv/build.ninja
    # The link line of libmpv-2.dll: its static libraries (paths, -lNAME,
    # -l:FILE); gcc's own come from its directory.
    gccdir=$(dirname "$(gcc -print-libgcc-file-name)")
    awk '/^build libmpv-2.dll: /{f=1} f && /LINK_ARGS/{print; exit}' "$ninja" |
      grep -oE '"[^"]+"' | tr -d '"' | sed 's/^-Wl,//' | tr ',' '\n' |
      grep -E '^-l|^[^-].*\.a$' | sort -u | while read -r arg; do
        case $arg in
          -l:*) name=${arg#-l:} ;;
          -l*) name=lib${arg#-l}.a ;;
          *) name=$(basename "$arg")
             # Built by build-windows.sh from FFmpeg's and libaribcaption's
             # source; libvulkan-1.a is the loader package's import library.
             case $(cygpath -u "$arg") in
               "$(cygpath -u "$work")"/prefix/*)
                 [ "$name" = libvulkan-1.a ] && echo "$MINGW_PREFIX/lib/libvulkan-1.dll.a"
                 continue ;;
             esac ;;
        esac
        for dir in "$MINGW_PREFIX/lib" "$gccdir"; do
          [ -e "$dir/$name" ] && { echo "$dir/$name"; break; }
        done
      done | xargs pacman -Qoq | sort -u > "$LINKED.tmp"
    { echo "# packages whose libraries libmpv-2.dll links statically (setup-msys2.sh linked, $(date -u +%Y-%m-%d))"; cat "$LINKED.tmp"; } > "$LINKED"
    rm -f "$LINKED.tmp"
    echo "wrote $LINKED ($(grep -vc '^#' "$LINKED") packages)"
    ;;
  sources)
    dir=${2:?directory}
    mkdir -p "$dir"
    "$0" check >/dev/null
    [ "${3:-}" != lock ] || echo "# source package file sha256 (setup-msys2.sh sources <dir> lock, $(date -u +%Y-%m-%d))" > "$SOURCES_LOCK"
    grep -v '^#' "$LINKED" | while read -r name; do
      version=$(pacman -Q "$name" | cut -d' ' -f2)
      base=$(sed -n '/^%BASE%$/{n;p;q}' /var/lib/pacman/local/"$name-$version"/desc)
      file=$base-$version.src.tar.zst
      # Several packages of one base (gcc, gcc-libs, libgomp) share it.
      [ -f "$dir/$file" ] || curl -fsSL -o "$dir/$file" "$REPO/mingw/sources/$file"
      if [ "${3:-}" = lock ]; then
        grep -q " $file " "$SOURCES_LOCK" || echo "$name $file $(sha256sum "$dir/$file" | cut -d' ' -f1)" >> "$SOURCES_LOCK"
      else
        sha=$(awk -v f="$file" '$2 == f { print $3; exit }' "$SOURCES_LOCK")
        [ -n "$sha" ] || { echo "$file is not in $SOURCES_LOCK" >&2; exit 1; }
        echo "$sha  $dir/$file" | sha256sum -c - >/dev/null
      fi
    done
    echo "$(ls "$dir" | wc -l) source packages in $dir"
    ;;
  *) echo "usage: setup-msys2.sh lock | install [dir] | archive <dir> | check | linked <work dir> | sources <dir> [lock]"; exit 2 ;;
esac
