#!/usr/bin/env bash
# release.yml: copy each installer in DIR to a fixed name without the
# version, so the download page can link to
#   https://github.com/<owner>/<repo>/releases/latest/download/<fixed name>
# Usage: stable-names.sh DIR VERSION
#
# VestraPlayer-<VERSION>-<platform><kind> → VestraPlayer-<platform><kind>,
# for the installer kinds below; <platform> is what bundle-*.mjs passes to
# artifactBase (windows-x64, macos-arm64; Linux later):
#   VestraPlayer-windows-x64-setup.exe
#   VestraPlayer-macos-arm64.dmg
#   VestraPlayer-linux-x86_64.AppImage   (reserved; nothing builds it yet)
# The portable zip and the .app.zip keep only their versioned names. Run it
# on the files that are uploaded (after signing), and before SHA256SUMS is
# written or checked: the copies are listed there too, so a file fetched by
# its fixed name can be checked with `sha256sum -c --ignore-missing`.
# Idempotent: an existing copy is replaced. Fails when no installer is found.
set -euo pipefail

dir=${1:?usage: stable-names.sh DIR VERSION}
version=${2:?usage: stable-names.sh DIR VERSION}
prefix="VestraPlayer-$version-"

copied=0
for path in "$dir"/"$prefix"*; do
  [ -f "$path" ] || continue
  file=${path##*/}
  case "$file" in
    *-setup.exe | *.dmg | *.AppImage) ;;
    *) continue ;;
  esac
  stable="VestraPlayer-${file#"$prefix"}"
  cp -f -- "$path" "$dir/$stable"
  echo "$file → $stable"
  copied=$((copied + 1))
done

if [ "$copied" = 0 ]; then
  echo "::error::no VestraPlayer-$version-* installer in $dir"
  exit 1
fi
