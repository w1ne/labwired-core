#!/usr/bin/env bash
# Keep labwired-core's canary focused on surfaces this repository owns — and
# make its fetches survive a transient failure of those surfaces.
#
# The canary files an issue titled "a published install path is broken" on any
# red. On 2026-09-25 it filed one because GitHub Pages answered a single 500
# for mcp.ps1, while the sibling PowerShell 7 leg fetched the same URL two
# seconds earlier and every Linux leg fetched the same host in the same window.
# A path that was healthy seconds later was reported as broken because the
# fetches had no retry, and — on Windows — because `irm | iex` cannot tell
# "the fetch failed" apart from "the installer failed".
#
# So this gate requires three things, and proves it can say no to each:
#   1. every fetch of a published path retries a transient failure;
#   2. a fetched body is checked to BE the script before it is executed;
#   3. the canary stays out of surfaces another repository owns.
#
# grep, not ripgrep: the Ubuntu runner image does not carry `rg`, and the
# previous form of this gate — `! rg -q '…'` — read "command not found" as
# "no violation" and passed vacuously on every PR. A gate that cannot run must
# not pass, and a tool every runner is guaranteed to have is the fix.
#
# Usage: scripts/ci/test-install-canary-boundaries.sh [workflow]
set -euo pipefail

workflow=${1:-.github/workflows/install-canary.yml}
test -f "$workflow"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

count_matches() {
  local file=$1 pattern=$2
  grep -c -e "$pattern" "$file" 2>/dev/null || true
}

# check <workflow> — print every contract violation, non-zero when any exist.
check() {
  local file=$1 bad=0 line fetches checks

  if grep -Eq 'marketplace\.visualstudio\.com|open-vsx\.org|labwired-vscode|extension is installable' "$file"; then
    printf '  the core canary owns the VS Code extension registry again\n' >&2
    bad=1
  fi

  # 1. No fetch of the landing site without curl's retry contract. `--retry`
  #    retries 5xx and timeouts; `-f` keeps a 4xx failing immediately, because
  #    a 404 is a real broken path and must not be papered over by a retry.
  while IFS= read -r line; do
    printf '  a published fetch without curl retry: %s\n' "$line" >&2
    bad=1
  done < <(grep -En 'curl[^|]*https://labwired\.com' "$file" | grep -v -- '--retry' || true)

  # A Windows leg may not execute whatever the network handed it: a fetch
  # failure and a broken installer are different failures and must read
  # differently in the log.
  if grep -Eq 'irm[[:space:]]+https://labwired\.com/mcp\.ps1[[:space:]]*\|[[:space:]]*iex' "$file"; then
    printf '  a Windows leg still pipes irm straight into iex\n' >&2
    bad=1
  fi

  # 2. Both PowerShell legs fetch through the retrying helper and refuse a
  #    body that is not the installer before executing it. Two, because the
  #    point of the job is that both Windows PowerShell 5.1 and PowerShell 7
  #    get the same treatment.
  fetches=$(count_matches "$file" 'function Get-Published')
  checks=$(count_matches "$file" "notmatch 'LabWired MCP'")
  if [ "${fetches:-0}" -lt 2 ]; then
    printf '  expected both Windows legs to fetch through a retrying helper; found %s\n' "${fetches:-0}" >&2
    bad=1
  fi
  if [ "${checks:-0}" -lt 2 ]; then
    printf '  expected both Windows legs to refuse a body that is not the installer; found %s\n' "${checks:-0}" >&2
    bad=1
  fi

  # mcp.sh is fetched to a file and run, so it gets the same body check.
  if grep -q 'https://labwired\.com/mcp\.sh' "$file" \
    && ! grep -q "grep -q 'LabWired MCP' mcp\.sh" "$file"; then
    printf '  mcp.sh is fetched and executed without checking it is the installer\n' >&2
    bad=1
  fi

  return "$bad"
}

# expect <pass|fail> <description> <workflow>
expect() {
  local want=$1 desc=$2 file=$3 got
  if check "$file" >/dev/null 2>&1; then got=pass; else got=fail; fi
  if [ "$got" = "$want" ]; then
    printf 'ok    %s\n' "$desc"
  else
    printf 'FAIL  %s — the gate said %s, wanted %s\n' "$desc" "$got" "$want" >&2
    failures=$((failures + 1))
  fi
}

# The positive control. If the real workflow does not carry the contract, the
# rest of the cases prove nothing.
expect pass "the real canary retries its fetches and checks their bodies" "$workflow"

# Mutations of the real workflow, each breaking one rule. A gate that cannot
# name a broken workflow is a gate that will not name a broken change.
sed 's/--retry 3 --retry-delay 2 //g' "$workflow" > "$work/no-retry.yml"
expect fail "a published fetch without --retry is rejected" "$work/no-retry.yml"

sed "s/\$script = Get-Published 'https:\/\/labwired.com\/mcp.ps1'/\$script = irm https:\/\/labwired.com\/mcp.ps1 | iex/" \
  "$workflow" > "$work/direct-irm.yml"
expect fail "a Windows leg that pipes irm straight into iex is rejected" "$work/direct-irm.yml"

sed "/notmatch 'LabWired MCP'/d; /grep -q 'LabWired MCP' mcp.sh/d" \
  "$workflow" > "$work/no-body-check.yml"
expect fail "a fetched body executed without a body check is rejected" "$work/no-body-check.yml"

{ cat "$workflow"; printf '\n# extension is installable\n'; } > "$work/scope.yml"
expect fail "the canary taking another repository's surface is rejected" "$work/scope.yml"

if [ "$failures" -ne 0 ]; then
  printf '\n%d canary boundary case(s) failed.\n' "$failures" >&2
  exit 1
fi
printf '\nThe canary boundaries hold, and the gate fails when they do not.\n'
