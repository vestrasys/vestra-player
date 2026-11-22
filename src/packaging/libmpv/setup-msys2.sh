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
#
# Run in an MSYS2 UCRT64 shell.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
LOCK=$HERE/msys2-packages.lock
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
  *) echo "usage: setup-msys2.sh lock | install [dir] | archive <dir> | check"; exit 2 ;;
esac
