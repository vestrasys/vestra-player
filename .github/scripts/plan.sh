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
# A built-in plugin's README / CHANGELOG ship in its package, so a change
# under src/plugins/ is never docs only: static runs that plugin's tests.
docs_only=true
if [ "$all" = true ] || printf '%s\n' "$files" | grep -Ev '^$|^doc/|\.md$' | grep -q . \
  || printf '%s\n' "$files" | grep -q '^src/plugins/'; then
  docs_only=false
fi

# Windows: every host change (nobody can run it locally). macOS: on main and
# on request (full; PRs: ci-macos.yml) — branches are built on a Mac anyway
# (npm run ci:local).
run_windows=false; { [ "$host" = true ] || [ "$FULL" = true ]; } && run_windows=true
run_macos=false
{ [ "$FULL" = true ] || { [ "$REF" = refs/heads/main ] && [ "$host" = true ]; }; } && run_macos=true
if [ "$docs_only" = true ]; then run_windows=false; run_macos=false; lock=false; fi

# Plugin tests (the ts job of _static.yml). A plugin imports only its own
# files and the SDK (npm run boundaries), so a change inside one plugin's
# directory tests that plugin; a change anywhere else except src/host/ (the
# SDK version, scripts, CI, ...) tests every plugin.
#
# The plugin tests also record the values the plugins send to their Slint
# views, which the Windows build-test checks against the compiled Slint
# (_build-test.yml `ui-record`). The host's Slint library (`@vestra/*`), its
# value conversion and the Slint version (Cargo.lock) are part of that check,
# so a change to them tests (and records) every plugin.
host_ui='^src/host/(ui/|src/(ui|ui_values|ui_types|ui_image|ui_markdown[a-z_]*|composition)\.rs$|src/i18n/|Cargo\.lock$)'
plugins=none
if [ "$all" = true ] || has "$host_ui" \
  || printf '%s\n' "$files" | grep -Ev '^$|^doc/|\.md$|^src/host/|^src/plugins/[^/]+/' | grep -q .; then
  plugins=all
else
  names=""
  for name in $(printf '%s\n' "$files" | sed -n 's|^src/plugins/\([a-z0-9-]*\)/.*|\1|p' | sort -u); do
    [ -f "src/plugins/$name/manifest.json" ] && names="$names $name"
  done
  [ -n "$names" ] && plugins="${names# }"
fi

# The recorded props check runs in the Windows build-test, or on its own
# (ci.yml/main.yml `ui-props`) when Windows does not run; see below for main.
ui_props=true
[ "$plugins" = none ] && ui_props=false

# main only: a squash merge of an up-to-date pull request has the same tree as
# its head. When a branch run already passed the Windows or macOS build-test
# on exactly this tree (ci.yml windows, ci.yml full or ci-macos.yml macos),
# main does not build it again. ci-macos.yml tests the PR's merge commit,
# whose tree is the head's when the head contains the base, which is the only
# way main can end up with the head's tree.
if [ "$REF" = refs/heads/main ] && [ "$EVENT" = push ] && [ -n "${GH_TOKEN:-}" ] \
  && { [ "$run_windows" = true ] || [ "$run_macos" = true ] || [ "$plugins" != none ]; }; then
  tree="$(git rev-parse 'HEAD^{tree}')"
  passed="$(
    for wf in ci.yml ci-macos.yml; do
      gh api "repos/$REPO/actions/workflows/$wf/runs?status=completed&per_page=50" \
        --jq ".workflow_runs[] | select(.head_commit.tree_id == \"$tree\") | .id" 2>/dev/null || true
    done | while read -r id; do
      gh api "repos/$REPO/actions/runs/$id/jobs?per_page=100" \
        --jq ".jobs[] | select(.conclusion == \"success\") | \"\\(.name)\t$id\"" 2>/dev/null || true
    done
  )"
  # The branch run that passed on this tree also checked the recorded props
  # (in its Windows build-test, or its own ui-props job).
  if [ "$ui_props" = true ] && printf '%s\n' "$passed" | awk -F'\t' '$1 == "windows / windows" || $1 == "ui-props / windows" { f = 1 } END { exit !f }'; then
    echo "ui-props: tree $tree already checked on a branch; skipped"
    ui_props=false
  fi
  for os in windows macos; do
    run_var="run_$os"
    [ "${!run_var}" = true ] || continue
    id="$(printf '%s\n' "$passed" | awk -F'\t' -v job="$os / $os" '$1 == job { print $2; exit }')"
    if [ -n "$id" ]; then
      echo "$os: tree $tree already passed in https://github.com/$REPO/actions/runs/$id; skipped"
      printf -v "$run_var" false
    fi
  done
fi

{
  echo "docs_only=$docs_only"
  echo "host=$host"
  echo "lock=$lock"
  echo "run_windows=$run_windows"
  echo "run_macos=$run_macos"
  echo "plugins=$plugins"
  echo "ui_props=$ui_props"
} | tee -a "${GITHUB_OUTPUT:-/dev/null}"
