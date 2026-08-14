#!/usr/bin/env bash
# Prove the chips gate can fail.
#
# The first version of that gate would have passed a chip whose run printed
# nothing and faulted silently, because it judged "no output and exit 0" as
# success — which is what a Cortex-M4 image does on an M0+ part. A gate nobody
# has watched fail is a gate nobody should trust, so this hands it three broken
# maps and requires a non-zero exit from each.
#
# Usage: scripts/ci/test-docs-runnable-chips.sh [path-to-labwired]
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1
CLI="${1:-labwired}"
GATE=scripts/ci/docs-runnable-chips.sh
MAP=scripts/ci/docs-runnable-chips.json
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

expect_gate_to_fail() {
  local description="$1"
  local map="$2"
  if LABWIRED_CHIPS_MAP="$map" bash "$GATE" "$CLI" > "$work/out" 2>&1; then
    printf 'FAIL  %s — the gate passed\n' "$description" >&2
    sed -n '1,12p' "$work/out" >&2
    failures=$((failures + 1))
  else
    printf 'ok    %s\n' "$description"
  fi
}

# 1. Firmware built for another part: it loads, prints nothing, and the CPU
#    faults. Exit status alone says success here, which is the whole point.
python3 - "$MAP" "$work/wrong-firmware.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m['chips']['stm32l073'] = {
    'how': 'committed',
    'firmware': 'tests/fixtures/tier1/stm32f407.elf',
    'expect': 'TIER1 clock PASS',
}
json.dump(m, open(sys.argv[2], 'w'), indent=2)
PY
expect_gate_to_fail "a chip paired with another part's firmware fails" "$work/wrong-firmware.json"

# 2. A chip that ships but is absent from the map must not pass silently.
python3 - "$MAP" "$work/missing-chip.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
del m['chips']['stm32h563']
json.dump(m, open(sys.argv[2], 'w'), indent=2)
PY
expect_gate_to_fail "a chip missing from the map fails" "$work/missing-chip.json"

# 3. A marker the firmware never prints. Catches an entry that was copied from
#    a sibling chip and never actually run.
python3 - "$MAP" "$work/wrong-marker.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m['chips']['stm32f103']['expect'] = 'TIER1 THIS LINE IS NEVER PRINTED'
json.dump(m, open(sys.argv[2], 'w'), indent=2)
PY
expect_gate_to_fail "an expectation the firmware never prints fails" "$work/wrong-marker.json"

# And the real map still passes, so the checks above are not simply "everything fails".
if bash "$GATE" "$CLI" > "$work/real" 2>&1; then
  printf 'ok    the committed map passes\n'
else
  printf 'FAIL  the committed map does not pass\n' >&2
  tail -n 12 "$work/real" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  printf '\n%d self-test(s) failed — the chips gate is not trustworthy as written.\n' "$failures" >&2
  exit 1
fi
printf '\nThe chips gate fails when it should, and passes when it should.\n'
