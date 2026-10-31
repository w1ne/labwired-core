#!/usr/bin/env bash
# LPC1114FN28/102 CLI proof: boot the smoke, assert the UART/GPIO stream, then
# require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/lpc1114fn28-dip
cargo run -q -p labwired-cli -- test \
  --script examples/lpc1114fn28-dip/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv6m-none-eabi/release/firmware-lpc1114fn28-smoke \
  --system configs/systems/lpc1114fn28-dip.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
