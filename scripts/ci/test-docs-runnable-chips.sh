#!/usr/bin/env bash
# Prove the chips gate can fail.
#
# The first version of that gate would have passed a chip whose run printed
# nothing and faulted silently, because it read "no output, exit 0" as success
# — which is exactly what a Cortex-M4 image does on an M0+ part. A gate nobody
# has watched fail is a gate nobody should trust, so this hands it three broken
# maps and requires a non-zero exit from each, plus one correct map that must
# pass so the three are not satisfied by a gate that simply fails at everything.
#
# Each case uses a cut-down map: the real chip list, with everything outside the
# chip under test parked in `needs-build`, which runs nothing. Executing 26
# chips four times over would take ten minutes to answer what two chips answer
# in seconds, and the full map is already exercised by the install canary.
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

python3 - "$MAP" "$work" <<'PY'
import json, pathlib, sys

src, work = sys.argv[1], pathlib.Path(sys.argv[2])
real = json.load(open(src))


def trimmed(keep):
    """The real map with every chip outside `keep` parked in needs-build.

    The chip list stays complete on purpose: the gate fails when a shipped
    chip descriptor has no entry, and these fixtures must not trip that rule
    by accident instead of the rule under test.
    """
    m = json.loads(json.dumps(real))
    for chip in list(m["chips"]):
        if chip not in keep:
            m["chips"][chip] = {"how": "needs-build", "why": "not under test here"}
    return m


# 1. Firmware built for another part: it loads, prints nothing, and the CPU
#    faults. Exit status alone reads as success here, which is the point.
m = trimmed({"stm32l073"})
m["chips"]["stm32l073"] = {
    "how": "committed",
    "firmware": "tests/fixtures/tier1/stm32f407.elf",
    "expect": "TIER1 clock PASS",
}
json.dump(m, open(work / "wrong-firmware.json", "w"), indent=2)

# 2. A chip that ships but is absent from the map.
m = trimmed({"stm32f103"})
del m["chips"]["stm32h563"]
json.dump(m, open(work / "missing-chip.json", "w"), indent=2)

# 3. A marker the firmware never prints — an entry copied from a sibling chip
#    and never actually run.
m = trimmed({"stm32f103"})
m["chips"]["stm32f103"]["expect"] = "TIER1 THIS LINE IS NEVER PRINTED"
json.dump(m, open(work / "wrong-marker.json", "w"), indent=2)

# 4. The positive control.
json.dump(trimmed({"stm32f103"}), open(work / "good.json", "w"), indent=2)
PY

expect_gate_to_fail() {
  description="$1"
  map="$2"
  if LABWIRED_CHIPS_MAP="$map" bash "$GATE" "$CLI" > "$work/out" 2>&1; then
    printf 'FAIL  %s — the gate passed\n' "$description" >&2
    sed -n '1,12p' "$work/out" >&2
    failures=$((failures + 1))
  else
    printf 'ok    %s\n' "$description"
  fi
}

expect_gate_to_fail "a chip paired with another part's firmware fails" "$work/wrong-firmware.json"
expect_gate_to_fail "a chip missing from the map fails"                "$work/missing-chip.json"
expect_gate_to_fail "an expectation the firmware never prints fails"   "$work/wrong-marker.json"

if LABWIRED_CHIPS_MAP="$work/good.json" bash "$GATE" "$CLI" > "$work/real" 2>&1; then
  printf 'ok    a correct map passes\n'
else
  printf 'FAIL  a correct map does not pass\n' >&2
  tail -n 12 "$work/real" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  printf '\n%d self-test(s) failed — the chips gate is not trustworthy as written.\n' "$failures" >&2
  exit 1
fi
printf '\nThe chips gate fails when it should, and passes when it should.\n'
