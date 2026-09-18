#!/usr/bin/env bash
# Run MemBrowse and LabWired over the same committed ELFs and merge the results.
#
# Both halves run locally and need no account: MemBrowse's local mode writes JSON
# to disk instead of uploading, and `labwired test` runs free-tier. Upload and PR
# comments are CI concerns — see github-actions.yml.
#
#   ./run-demo.sh              # both targets
#   ./run-demo.sh nrf54l15-dk  # one target
#
# Requires: membrowse (pip install membrowse pyyaml) and labwired on PATH
# (curl -fsSL https://labwired.com/install.sh | sh).
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${OUT_DIR:-$ROOT/out/membrowse-demo}"

# target | elf | linker script | labwired test script
TARGETS=(
  "nrf54l15-dk|examples/nrf54l15-dk/build/nrf54l15-smoke.elf|examples/nrf54l15-dk/nrf54l15.ld|examples/nrf54l15-dk/io-smoke.yaml"
  "esp32c3-blinky|examples/esp32c3-blinky/firmware/esp32c3_blinky.elf|examples/esp32c3-blinky/firmware/c3.ld|examples/esp32c3-blinky/test-blink.yaml"
)

for tool in membrowse labwired; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "error: $tool not found on PATH (see the header of this script)" >&2
    exit 2
  }
done

cd "$ROOT"
status=0

for entry in "${TARGETS[@]}"; do
  IFS='|' read -r name elf ld script <<<"$entry"
  [[ $# -gt 0 && "$1" != "$name" ]] && continue

  echo "==> $name"
  mkdir -p "$OUT/$name"

  # Static half: every byte the linker placed, attributed to a symbol and a file.
  membrowse report "$elf" "$ld" --json >"$OUT/$name/membrowse.json"

  # Runtime half: the same ELF executed on the modeled chip. Exits non-zero on a
  # failed assertion; let the combined report render that rather than aborting.
  labwired test \
    --script "$script" \
    --output-dir "$OUT/$name/labwired" \
    --no-uart-stdout >"$OUT/$name/labwired.log" 2>&1 || true

  python3 "$HERE/combined-report.py" \
    --target "$name" \
    --membrowse "$OUT/$name/membrowse.json" \
    --labwired "$OUT/$name/labwired/result.json" \
    --budgets "$HERE/budgets.yaml" \
    --markdown "$OUT/$name/combined.md" \
    --json "$OUT/$name/combined.json" || status=1
  echo
done

echo "artifacts: $OUT"
exit "$status"
