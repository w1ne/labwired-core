#!/usr/bin/env bash
# How far the shipped CLI has drifted from the tree that documents it.
#
# On 2026-08-14 main was 984 commits and 18 days ahead of v0.21.0. Every one of
# those commits improved something; none of them reached anybody. The examples
# in the repo had moved on to script fields the released CLI could not parse,
# `esp32c3` could not run at all on the last release while working perfectly on
# main, and `configs/chips/stm32l073.yaml` used a schema the released binary
# rejected outright. All three read like product defects and were release lag.
#
# Nothing measured that, because staleness is the one property no test in the
# repo can see: every suite runs against the working tree, which is by
# definition current. So it gets measured here, against the tags.
#
# This runs in the install canary rather than on pull requests. A PR did not
# cause the drift and cannot fix it; a scheduled red is the honest shape — the
# repo is telling you a release is overdue, not that your change is wrong.
#
# Usage: scripts/ci/release-freshness.sh [max_days] [max_commits]
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1

MAX_DAYS="${1:-21}"
MAX_COMMITS="${2:-250}"

# Only vMAJOR.MINOR.PATCH counts. Demo-firmware tags are not releases of the
# CLI, and treating one as the latest release is exactly what broke `install.sh`.
latest_tag="$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname | head -1)"
if [ -z "$latest_tag" ]; then
  echo "::error::no vMAJOR.MINOR.PATCH tag found — is this a shallow clone? Fetch tags before running this."
  exit 1
fi

tag_epoch="$(git log -1 --format=%ct "$latest_tag")"
now_epoch="$(date -u +%s)"
days=$(( (now_epoch - tag_epoch) / 86400 ))
commits="$(git rev-list --count "${latest_tag}..HEAD")"

printf 'latest release   %s\n' "$latest_tag"
printf 'age              %s days (limit %s)\n' "$days" "$MAX_DAYS"
printf 'commits since    %s (limit %s)\n' "$commits" "$MAX_COMMITS"

fail=0
if [ "$days" -gt "$MAX_DAYS" ]; then
  echo "::error::${latest_tag} is ${days} days old. Anyone installing LabWired today gets a CLI that predates ${commits} commits of this tree."
  fail=1
fi
if [ "$commits" -gt "$MAX_COMMITS" ]; then
  echo "::error::${commits} commits since ${latest_tag}. The examples and docs in this repo describe a CLI nobody can install."
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "Cut a release: bump the workspace version, then tag. RELEASE_PROCESS.md has the two-commit ritual."
  exit 1
fi

echo
echo "The shipped CLI is close enough to this tree that the docs describe it."
