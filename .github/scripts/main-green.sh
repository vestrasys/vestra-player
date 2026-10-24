#!/usr/bin/env bash
# release.yml / nightly.yml: has main.yml already checked this commit?
# Inputs (env): SHA (the commit), REPO, GH_TOKEN (actions: read), WAIT
# (seconds to wait for a main.yml run of SHA that has not finished yet).
# Needs the full history (fetch-depth: 0). Writes tested=true|false to
# $GITHUB_OUTPUT.
#
# A green main.yml run covers every change since the previous green one
# (plan.sh), and main runs everything when there is none, so a green run on
# commit S means static and both build-tests passed for S's tree. SHA counts
# as checked when such an S is SHA itself, or an ancestor of it with only
# documentation (doc/, *.md) changed in between: a tag on the release-notes
# commit, for example.
set -euo pipefail

runs="repos/$REPO/actions/workflows/main.yml/runs"
deadline=$(( $(date +%s) + ${WAIT:-0} ))
while :; do
  pending="$(gh api "$runs?head_sha=$SHA&per_page=20" \
    --jq '[.workflow_runs[] | select(.status != "completed")] | length')"
  [ "$pending" = 0 ] && break
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "main.yml has not finished on $SHA yet; not waiting longer"
    break
  fi
  echo "waiting for main.yml on $SHA ($pending run(s) not finished)"
  sleep 30
done

tested=false
for s in $(gh api "$runs?branch=main&status=success&per_page=30" --jq '.workflow_runs[].head_sha'); do
  git cat-file -e "$s^{commit}" 2>/dev/null || continue
  git merge-base --is-ancestor "$s" "$SHA" || continue
  other="$(git diff --name-only "$s" "$SHA" | grep -Ev '^doc/|\.md$' || true)"
  if [ "$s" = "$SHA" ]; then
    echo "main.yml passed $SHA"
    tested=true
  elif [ -z "$other" ]; then
    echo "main.yml passed $s; only documentation changed from there to $SHA"
    tested=true
  else
    echo "main.yml last passed $s, which is not $SHA and code changed in between:"
    printf '%s\n' "$other" | sed 's/^/  /'
  fi
  break
done
echo "tested=$tested" | tee -a "${GITHUB_OUTPUT:-/dev/null}"
