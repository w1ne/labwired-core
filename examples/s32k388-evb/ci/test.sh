#!/usr/bin/env bash
# S32K388EVB-Q289 CLI proof: boot the S32K388 smoke, assert the LPUART2/SIUL2
# LED/button stream, then require the firmware's instruction stream to be
# fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/s32k388-evb
cargo run -q -p labwired-cli -- test \
  --script examples/s32k388-evb/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-s32k388-evb-smoke \
  --system configs/systems/s32k388-evb.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
