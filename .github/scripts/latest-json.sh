#!/usr/bin/env bash
# release.yml: write or check latest.json, the release's description of
# itself, read from the stable URL
#   https://github.com/<owner>/<repo>/releases/latest/download/latest.json
# by the player's update check and by the download page at build time
# (doc/releases/README.md "latest.json"; precedent: Tauri's latest.json,
# electron-updater's latest.yml).
#
#   latest-json.sh write DIR VERSION REPO   writes DIR/latest.json
#   latest-json.sh check DIR VERSION REPO   validates DIR/latest.json
#
# Run after stable-names.sh, on the files that are uploaded: `downloads`
# lists each fixed-name installer in DIR (VestraPlayer-<platform><kind>)
# under its <platform> key, with the size and SHA-256 of that file, so a
# client fetches releases/latest/download/<file>. `minimumOs` comes from the
# packaging, never from a constant here: windows-* from MinVersion in
# src/packaging/windows/vestra.iss, macos-* from LSMinimumSystemVersion in
# the Info.plist of the same build's VestraPlayer-<VERSION>-<platform>.app.zip
# (bundle-macos.mjs computes it from the dependency bundle); linux-* has none
# yet. `published` is now in UTC, or $PUBLISHED when set (tests).
# Needs jq, sha256sum, stat, unzip (all on ubuntu-24.04 runners).
set -euo pipefail

usage="usage: latest-json.sh write|check DIR VERSION REPO"
mode=${1:?$usage}
dir=${2:?$usage}
version=${3:?$usage}
repo=${4:?$usage}
out="$dir/latest.json"
notes="https://github.com/$repo/releases/tag/v$version"
iss="$(dirname "$0")/../../src/packaging/windows/vestra.iss"

fail() {
  echo "::error::latest.json: $*"
  exit 1
}

# Fixed name → platform key, or nothing when it is not an installer.
platform_of() {
  local name=${1#VestraPlayer-}
  [ "$name" != "$1" ] || return 0
  case "$name" in
    *-setup.exe) echo "${name%-setup.exe}" ;;
    *.dmg) echo "${name%.dmg}" ;;
    *.AppImage) echo "${name%.AppImage}" ;;
  esac
}

# Sets $minimum for platform $1 ("" when the packaging defines none). Not
# called in $(…), so that fail ends the script and its message is shown.
minimum_os() {
  local platform=$1 zip value=""
  case "$platform" in
    windows-*)
      [ -f "$iss" ] || fail "$iss is missing (checkout src/packaging/windows)"
      value=$(sed -n 's/^MinVersion=\([0-9.]*\)\r*$/\1/p' "$iss" | head -1)
      [ -n "$value" ] || fail "no MinVersion in $iss"
      ;;
    macos-*)
      zip="$dir/VestraPlayer-$version-$platform.app.zip"
      [ -f "$zip" ] || fail "${zip##*/} is missing (LSMinimumSystemVersion of $platform)"
      # unzip fails when the zip has no Info.plist; "|| true" keeps set -e and
      # pipefail from ending the script here without a message, so the fail
      # below says why.
      value=$( (unzip -p "$zip" "Vestra Player.app/Contents/Info.plist" || true) |
        tr -d '\r' |
        sed -n '/<key>LSMinimumSystemVersion<\/key>/{n;s/.*<string>\([0-9.]*\)<\/string>.*/\1/p;}' |
        head -1)
      [ -n "$value" ] || fail "no LSMinimumSystemVersion in ${zip##*/}"
      ;;
  esac
  minimum=$value
}

write() {
  local published=${PUBLISHED:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}
  local downloads='{}' path file platform size sha minimum
  for path in "$dir"/VestraPlayer-*; do
    [ -f "$path" ] || continue
    file=${path##*/}
    platform=$(platform_of "$file")
    # Versioned names (VestraPlayer-<VERSION>-…) are not fixed names.
    case "$platform" in "" | "$version"-*) continue ;; esac
    size=$(stat -c %s -- "$path")
    sha=$(sha256sum -- "$path" | cut -d' ' -f1)
    minimum_os "$platform"
    downloads=$(jq -c --arg p "$platform" --arg f "$file" --argjson s "$size" \
      --arg h "$sha" --arg m "$minimum" \
      '.[$p] = ({file: $f, size: $s, sha256: $h} + (if $m == "" then {} else {minimumOs: $m} end))' \
      <<< "$downloads")
  done
  [ "$downloads" != '{}' ] || fail "no fixed-name installer in $dir (run stable-names.sh first)"
  jq -n --arg v "$version" --arg p "$published" --arg n "$notes" --argjson d "$downloads" \
    '{schema: 1, version: $v, published: $p, notes: $n, downloads: $d}' > "$out"
  cat "$out"
}

check() {
  local key file size sha min path minimum
  [ -f "$out" ] || fail "$out is missing"
  jq -e . "$out" > /dev/null || fail "$out does not parse"
  jq -e '.schema == 1' "$out" > /dev/null || fail "schema is not 1"
  jq -e --arg v "$version" '.version == $v' "$out" > /dev/null ||
    fail "version $(jq -c .version "$out") is not the tag's $version"
  jq -e --arg n "$notes" '.notes == $n' "$out" > /dev/null || fail "notes is not $notes"
  jq -e '.published | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")' \
    "$out" > /dev/null || fail "published is not a UTC ISO time"
  jq -e '.downloads | type == "object" and length > 0' "$out" > /dev/null || fail "downloads is empty"
  while IFS=$'\t' read -r key file size sha min; do
    [ "$(platform_of "$file")" = "$key" ] || fail "$key: $file is not VestraPlayer-$key<kind>"
    case "$key" in "$version"-* | */* | .*) fail "$key: not a platform" ;; esac
    path="$dir/$file"
    [ -f "$path" ] || fail "$key: $file is missing"
    [ "$(stat -c %s -- "$path")" = "$size" ] || fail "$key: $file size is not $size"
    [ "$(sha256sum -- "$path" | cut -d' ' -f1)" = "$sha" ] || fail "$key: $file sha256 differs"
    minimum_os "$key"
    [ "$min" = "$minimum" ] || fail "$key: minimumOs '$min' is not the packaging's '$minimum'"
    echo "latest.json: $key → $file ($size bytes) OK"
  done < <(jq -r '.downloads | to_entries[] | [.key, .value.file, (.value.size | tostring), .value.sha256, (.value.minimumOs // "")] | @tsv' "$out" | tr -d '\r')
}

case "$mode" in
  write) write ;;
  check) check ;;
  *) echo "$usage" >&2; exit 2 ;;
esac
