#!/usr/bin/env bash
# Automated Golden Reference Generator
# Usage: ./repro_golden_reference.sh <hw_trace.json> <firmware.elf> <system.yaml>

set -euo pipefail

HW_TRACE="${1:-}"
FIRMWARE="${2:-}"
SYSTEM="${3:-}"
TARGET="${4:-NUCLEO-H563ZI}"

if [[ -z "$HW_TRACE" || -z "$FIRMWARE" || -z "$SYSTEM" ]]; then
    echo "Usage: $0 <hw_trace.json> <firmware.elf> <system.yaml> [target_name]"
    exit 1
fi

OUT_DIR="out/golden-reference"
mkdir -p "$OUT_DIR"

SIM_TRACE="$OUT_DIR/sim_trace_repro.json"
REPORT="$OUT_DIR/determinism_report_repro.json"

echo "==> Generating Repo Simulator Trace..."
cargo run -q -p labwired-cli -- \
    --firmware "$FIRMWARE" \
    --system "$SYSTEM" \
    --trace "$SIM_TRACE" \
    --max-steps 1000

echo "==> Running Audit..."
./scripts/labwired-audit.py \
    --hw-trace "$HW_TRACE" \
    --sim-trace "$SIM_TRACE" \
    --target "$TARGET" \
    --firmware "$(basename "$FIRMWARE")" \
    --output "$REPORT" \
    --align-window 50

echo "==> Done. Report saved to $REPORT"
