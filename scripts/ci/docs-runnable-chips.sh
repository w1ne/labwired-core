#!/usr/bin/env bash
# Every chip we document, run with the CLI a user can actually install.
#
# The claim this defends is the one on the front page: pick a supported chip,
# run firmware on it, no cross toolchain. On 2026-08-14 that claim held for six
# chips out of twenty-four from a fresh clone — not because the models were
# broken, but because the firmware the examples point at is built in CI and
# never shipped, and because the last release was 984 commits behind the
# examples in the same tree.
#
# Rules, all enforced below:
#   * every chip descriptor in configs/chips (minus CI fixtures) must appear in
#     docs-runnable-chips.json — a new chip cannot arrive untested and silent;
#   * `committed`, `asset` and `example` entries are RUN, and judged on what the
#     firmware printed. Not on exit status: `labwired run` deliberately treats a
#     simulation error as a non-fatal end of run and still exits 0;
#   * `committed` is the preferred shape — the ELF is in this repo, so a clone
#     plus an installed CLI is the whole prerequisite, and most of them are the
#     TIER1 fixtures, whose transcript asserts real peripheral behaviour rather
#     than "something booted";
#   * prebuilt firmware is pinned by release tag AND sha256, so a re-uploaded
#     asset cannot quietly change what this proves;
#   * `blocked` entries are run too, and a blocked chip that now WORKS fails the
#     gate. A ratchet that only catches regressions leaves fixed things marked
#     broken forever.
#
# Usage: scripts/ci/docs-runnable-chips.sh [path-to-labwired]
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1
CLI="${1:-labwired}"
# The map is overridable so the gate can be tested against a deliberately
# broken one — a gate nobody has ever seen fail is a gate nobody should trust.
MAP="${LABWIRED_CHIPS_MAP:-scripts/ci/docs-runnable-chips.json}"
REPO=w1ne/labwired-core
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

command -v "$CLI" >/dev/null 2>&1 || [ -x "$CLI" ] || {
  echo "no labwired binary at '$CLI' — install one first:" >&2
  echo "  curl -fsSL https://labwired.com/install.sh | sh" >&2
  exit 1
}

# Fields are unit-separated, not tab-separated: a TAB is whitespace to `read`,
# which collapses runs of it, so two empty fields in a row silently become one.
read_map() { python3 -c "
import json,sys
m = json.load(open('$MAP'))
print(m['firmware_release'])
for chip, e in m['chips'].items():
    print('\x1f'.join([
        chip, e.get('how',''), e.get('firmware',''), e.get('sha256',''),
        e.get('script',''), e.get('expect') or '', e.get('why',''),
        e.get('release', m['firmware_release'])]))
"; }

# bash 3.2 on macOS has no `mapfile`, and the canary runs this on macOS.
read_map > "$work/map.tsv" || { echo "cannot read $MAP" >&2; exit 1; }
release_default="$(head -1 "$work/map.tsv")"
tail -n +2 "$work/map.tsv" > "$work/entries.tsv"

# ── The map has to cover the chips we ship ──────────────────────────────────
missing=""
for path in configs/chips/*.yaml; do
  chip="$(basename "$path" .yaml)"
  case "$chip" in ci-fixture-*) continue ;; esac
  cut -d$'\x1f' -f1 "$work/entries.tsv" | grep -qx "$chip" || missing="$missing $chip"
done
if [ -n "$missing" ]; then
  echo "::error::these chips ship but are absent from $MAP:$missing"
  echo "Add each one: how it runs from a clean install, or 'needs-build' with a reason."
  exit 1
fi

fetch_asset() {
  local name="$1"
  local want="$2"
  # Per-chip release override: firmware for a chip that was added later lives
  # on a later tag, and an existing tag is never re-uploaded — the playground
  # pins these same assets by tag and digest.
  local release="${3:-$release_default}"
  local dest="$work/$name"
  [ -f "$dest" ] && { printf '%s' "$dest"; return 0; }
  curl -fsSL --retry 3 -o "$dest" \
    "https://github.com/${REPO}/releases/download/${release}/${name}" || return 1
  local got
  got="$(shasum -a 256 "$dest" | cut -d' ' -f1)"
  if [ "$got" != "$want" ]; then
    echo "::error::$name in $release has sha256 $got, the map pins $want — the asset was replaced" >&2
    return 1
  fi
  printf '%s' "$dest"
}

# Judged on output, never on exit status alone.
run_and_judge() { # $1 chip, $2 how, $3 firmware, $4 sha, $5 script, $6 expect, $7 release → verdict
  local chip="$1"
  local how="$2"
  local fw="$3"
  local sha="$4"
  local script="$5"
  local expect="$6"
  local release="${7:-$release_default}"
  local out=""
  local path=""
  if [ "$how" = example ]; then
    out="$("$CLI" test --script "$script" --output-dir "$work/out-$chip" 2>&1)"
  elif [ "$how" = committed ]; then
    # Firmware that ships in this repo: no download, no digest to pin.
    if [ ! -f "$fw" ]; then echo "MISSING_FIXTURE"; return; fi
    out="$("$CLI" run --chip "configs/chips/${chip}.yaml" --firmware "$fw" --max-steps 6000000 2>&1)"
  else
    path="$(fetch_asset "$fw" "$sha" "$release")" || { echo "ASSET"; return; }
    out="$("$CLI" run --chip "configs/chips/${chip}.yaml" --firmware "$path" --max-steps 4000000 2>&1)"
  fi
  printf '%s' "$out" > "$work/log-$chip"
  grep -q "simulation error" "$work/log-$chip" && { echo "FAULT"; return; }
  grep -qE "^error:|failed to build system bus|cannot parse chip YAML" "$work/log-$chip" && { echo "REFUSED"; return; }
  # A bus fault or a CPU error is a failure even when the CLI keeps going and
  # prints nothing else; without this, pairing a chip with firmware built for a
  # different part reads as a silent pass.
  grep -qE "ERROR .*labwired_core" "$work/log-$chip" && { echo "CHIP_ERROR"; return; }
  if [ -n "$expect" ] && ! grep -qF -- "$expect" "$work/log-$chip"; then echo "SILENT"; return; fi
  echo "OK"
}

runnable=0; blocked=0; needs_build=0; failures=0
printf '%-16s %-12s %s\n' CHIP HOW RESULT
while IFS=$'\x1f' read -r chip how fw sha script expect why release; do
  case "$how" in
    committed|asset|example)
      verdict="$(run_and_judge "$chip" "$how" "$fw" "$sha" "$script" "$expect" "$release")"
      if [ "$verdict" = OK ]; then
        runnable=$((runnable + 1))
        printf '%-16s %-12s ok\n' "$chip" "$how"
      else
        failures=$((failures + 1))
        printf '%-16s %-12s FAILED (%s)\n' "$chip" "$how" "$verdict"
        sed -n '1,6p' "$work/log-$chip" 2>/dev/null | sed 's/^/      /'
        echo "::error::$chip is documented as runnable and is not: $verdict"
      fi
      ;;
    blocked)
      blocked=$((blocked + 1))
      verdict="$(run_and_judge "$chip" asset "$fw" "$sha" "" "$expect" "$release")"
      if [ "$verdict" = OK ]; then
        failures=$((failures + 1))
        printf '%-16s %-12s FIXED — promote it\n' "$chip" "$how"
        echo "::error::$chip is marked blocked but now runs. Move it to \"how\": \"asset\" in $MAP."
      else
        printf '%-16s %-12s still blocked (%s)\n' "$chip" "$how" "$verdict"
      fi
      ;;
    needs-build)
      needs_build=$((needs_build + 1))
      printf '%-16s %-12s no prebuilt firmware — %s\n' "$chip" "$how" "$why"
      ;;
    *)
      failures=$((failures + 1))
      echo "::error::$chip has an unknown \"how\": $how"
      ;;
  esac
done < "$work/entries.tsv"

total=$((runnable + blocked + needs_build))
echo
echo "$runnable of $total documented chips run from an install with no toolchain."
echo "$blocked blocked by the released CLI, $needs_build still need firmware built from source."
[ "$failures" -eq 0 ] || { echo; echo "$failures chip(s) do not match what this repo claims."; exit 1; }
