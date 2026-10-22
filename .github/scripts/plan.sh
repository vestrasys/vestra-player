#!/usr/bin/env bash
# ci.yml plan: which jobs this push / pull request needs (doc/process/ci.md).
# Inputs (env): EVENT, FULL, BEFORE, PR_BASE, BASE_REF. Writes to $GITHUB_OUTPUT.
set -euo pipefail

zero=0000000000000000000000000000000000000000
all=false
base=""
case "$EVENT" in
  pull_request) base="$PR_BASE" ;;
  workflow_dispatch) all=true ;;
  push)
    if [ -n "$BEFORE" ] && [ "$BEFORE" != "$zero" ] && git cat-file -e "$BEFORE^{commit}" 2>/dev/null; then
      base="$BEFORE"
    else
      # New branch or force push: everything since it left main.
      base="$(git merge-base origin/main HEAD 2>/dev/null || true)"
    fi
    ;;
  *) all=true ;;
esac
[ -z "$base" ] && all=true

if [ "$all" = true ]; then
  files="(all)"
else
  files="$(git diff --name-only "$base" HEAD)"
fi
echo "changed files (base ${base:-none}):"
printf '%s\n' "$files" | sed 's/^/  /'

has() { [ "$all" = true ] || printf '%s\n' "$files" | grep -Eq "$1"; }

# What the host build-test jobs depend on: the Rust workspace and the Node
# runtime its tests start (src/runtime, the SDK in src/package*.json). CI
# changes test everything they could affect.
host=false; has '^(src/host/|src/runtime/|src/package(-lock)?\.json$|rust-toolchain\.toml$|\.gitattributes$|\.github/)' && host=true
lock=false; has '^(src/host/Cargo\.lock|src/host/deny\.toml|\.github/workflows/_audit\.yml)$' && lock=true
docs_only=true
if [ "$all" = true ] || printf '%s\n' "$files" | grep -Ev '^$|^doc/|\.md$' | grep -q .; then
  docs_only=false
fi

run_windows=false; { [ "$host" = true ] || [ "$FULL" = true ]; } && run_windows=true
run_macos=false
{ [ "$FULL" = true ] || { [ "$EVENT" = pull_request ] && [ "$BASE_REF" = main ] && [ "$host" = true ]; }; } && run_macos=true
if [ "$docs_only" = true ]; then run_windows=false; run_macos=false; lock=false; fi

{
  echo "docs_only=$docs_only"
  echo "host=$host"
  echo "lock=$lock"
  echo "run_windows=$run_windows"
  echo "run_macos=$run_macos"
} | tee -a "$GITHUB_OUTPUT"
