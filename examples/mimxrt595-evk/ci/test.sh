#!/usr/bin/env bash
# MIMXRT595-EVK CLI proof: boot the MIMXRT595S smoke, assert the FLEXCOMM0/
# GPIO LED/switch stream, then require the firmware's instruction stream to be
# fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/mimxrt595-evk
cargo run -q -p labwired-cli -- test \
  --script examples/mimxrt595-evk/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-mimxrt595-evk-smoke \
  --system configs/systems/mimxrt595-evk.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
