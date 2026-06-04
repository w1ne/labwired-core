#!/usr/bin/env bash
# Run a firmware in the LabWired sim and diff its UART output against the
# hardware capture produced by hw-capture-stm32f103.sh.
#
# Usage:
#   bash core/scripts/diff-sim-vs-hw.sh <capture-dir> [firmware.elf]
#
# The capture dir must contain `firmware.elf` (or one passed on CLI) and
# `uart.log` (silicon ground truth).

set -euo pipefail

CAPTURE="${1:?capture dir required}"
FIRMWARE="${2:-$CAPTURE/firmware.elf}"
[ -f "$FIRMWARE" ] || { echo "firmware not found: $FIRMWARE"; exit 1; }
[ -f "$CAPTURE/uart.log" ] || { echo "no $CAPTURE/uart.log — capture first"; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LABWIRED_CLI="${LABWIRED_CLI:-$REPO_ROOT/core/target/release/labwired}"
[ -x "$LABWIRED_CLI" ] || { echo "build labwired-cli first: cargo build --release -p labwired-cli"; exit 1; }

SIM_OUT="$CAPTURE/sim-out"
mkdir -p "$SIM_OUT"

SCRIPT="$SIM_OUT/script.yaml"
cat > "$SCRIPT" <<EOF
schema_version: "1.0"
inputs:
  firmware: "$FIRMWARE"
  system: "$REPO_ROOT/core/examples/demo-blinky/system.yaml"
limits:
  max_steps: 5000000
  max_cycles: 5000000
  no_progress_steps: 10000
assertions: []
EOF

echo "==> Running sim..."
"$LABWIRED_CLI" test \
  --firmware "$FIRMWARE" \
  --system "$REPO_ROOT/core/examples/demo-blinky/system.yaml" \
  --script "$SCRIPT" \
  --output-dir "$SIM_OUT" \
  --max-cycles 5000000 \
  --no-uart-stdout

echo "==> sim result:"
jq -C '{status, stop_reason, cycles, steps_executed, message}' "$SIM_OUT/result.json" 2>/dev/null \
  || cat "$SIM_OUT/result.json"

echo ""
echo "==> diff hardware UART vs sim UART:"
HW_SIZE=$(wc -c < "$CAPTURE/uart.log")
SIM_SIZE=$(wc -c < "$SIM_OUT/uart.log" 2>/dev/null || echo 0)
echo "  hw:  $HW_SIZE bytes"
echo "  sim: $SIM_SIZE bytes"
echo ""

diff -u "$CAPTURE/uart.log" "$SIM_OUT/uart.log" || true

echo ""
echo "==> If diff non-empty: identify peripherals/registers the sim is wrong about,"
echo "    extend the chip yaml / peripheral model, rebuild, re-run this script."
