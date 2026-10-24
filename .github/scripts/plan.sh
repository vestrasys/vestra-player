#!/usr/bin/env bash
# ci.yml / main.yml plan: which jobs this run needs (doc/process/ci.md).
# Inputs (env): EVENT, FULL, REF (github.ref), WORKFLOW (file name, e.g.
# ci.yml), REPO, GH_TOKEN (actions: read). Writes to $GITHUB_OUTPUT.
#
# The changed files are counted from the last commit this workflow passed on
# this ref, not from the push's `before`: a push whose run was cancelled
# (cancel-in-progress, or a pending main run replaced by a newer one) or
# failed is then checked again by the next run, so a green ci-ok on the tip
# covers every change since the last green one. Without such a run: a branch
# counts from where it left main, main runs everything.
set -euo pipefail

all=false
base=""
branch="${REF#refs/heads/}"

last_green() {
  # Newest successful run of this workflow on this branch whose commit is an
  # ancestor of HEAD (a rebase or force push drops the older ones).
  [ -n "${GH_TOKEN:-}" ] && [ -n "${REPO:-}" ] && [ -n "${WORKFLOW:-}" ] || return 0
  local shas sha head
  head="$(git rev-parse HEAD)"
  shas="$(gh api "repos/$REPO/actions/workflows/$WORKFLOW/runs?branch=$branch&status=success&per_page=20" \
    --jq '.workflow_runs[].head_sha' 2>/dev/null || true)"
  for sha in $shas; do
    # A re-run of an already green commit checks again from the one before.
    [ "$sha" = "$head" ] && continue
    if git cat-file -e "$sha^{commit}" 2>/dev/null && git merge-base --is-ancestor "$sha" HEAD; then
      echo "$sha"
      return 0
    fi
  done
}

case "$EVENT" in
  push)
    base="$(last_green)"
    if [ -z "$base" ] && [ "$REF" != refs/heads/main ]; then
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
# changes test everything they could affect. Keep in step with the `paths`
# of ci-macos.yml.
host=false; has '^(src/host/|src/runtime/|src/package(-lock)?\.json$|rust-toolchain\.toml$|\.gitattributes$|\.github/)' && host=true
lock=false; has '^(src/host/Cargo\.lock|src/host/deny\.toml|\.github/workflows/_audit\.yml)$' && lock=true
docs_only=true
if [ "$all" = true ] || printf '%s\n' "$files" | grep -Ev '^$|^doc/|\.md$' | grep -q .; then
  docs_only=false
fi

# Windows: every host change (nobody can run it locally). macOS: on main and
# on request (full; PRs: ci-macos.yml) — branches are built on a Mac anyway
# (npm run ci:local).
run_windows=false; { [ "$host" = true ] || [ "$FULL" = true ]; } && run_windows=true
run_macos=false
{ [ "$FULL" = true ] || { [ "$REF" = refs/heads/main ] && [ "$host" = true ]; }; } && run_macos=true
if [ "$docs_only" = true ]; then run_windows=false; run_macos=false; lock=false; fi

{
  echo "docs_only=$docs_only"
  echo "host=$host"
  echo "lock=$lock"
  echo "run_windows=$run_windows"
  echo "run_macos=$run_macos"
} | tee -a "${GITHUB_OUTPUT:-/dev/null}"
