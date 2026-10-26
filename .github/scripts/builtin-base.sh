#!/usr/bin/env bash
# ci.yml / main.yml plan, release.yml verify: the commit the built-in
# versions guard (src/scripts/check-builtin-versions.mjs, _static.yml lint)
# compares HEAD against. Inputs (env): REF (github.ref). Needs the full
# history and the tags (fetch-depth: 0), which these jobs already have; the
# lint job then fetches only this one commit. Writes builtin_base=<sha> to
# $GITHUB_OUTPUT (empty when there is none; the lint step then fails).
#
# - A branch: where it left main (merge-base origin/main HEAD), so a branch
#   is judged by its own changes. Not origin/main itself: when main has
#   bumped and changed a plugin since the branch forked, the branch's older
#   copy would count as a change with a lower version.
# - main and v* tags: the latest v* tag before HEAD (described from HEAD^,
#   so a tag on HEAD itself, the release being cut, is skipped): what users
#   have. One bump per release is enough; a second change under an already
#   raised version still reaches users. main and release.yml give the same
#   answer for the tagged commit, so release can rely on main's green run.
set -euo pipefail

base=""
case "$REF" in
  refs/heads/main | refs/tags/*)
    if tag="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 'HEAD^' 2>/dev/null)"; then
      base="$(git rev-parse "$tag^{commit}")"
      echo "built-in versions base: $tag ($base), the latest release before HEAD"
    else
      echo "::warning::no v* tag before HEAD"
    fi
    ;;
  *)
    if base="$(git merge-base origin/main HEAD 2>/dev/null)"; then
      echo "built-in versions base: $base, where this branch left origin/main"
    else
      base=""
      echo "::warning::no merge base with origin/main"
    fi
    ;;
esac
echo "builtin_base=$base" >> "${GITHUB_OUTPUT:-/dev/null}"
