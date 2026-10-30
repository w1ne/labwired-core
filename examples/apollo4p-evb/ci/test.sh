#!/usr/bin/env bash
# Apollo4 Plus EVB CLI proof: boot the UART+LED smoke, assert the UART0/LED
# stream, then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/apollo4p-evb
cargo run -q -p labwired-cli -- test \
  --script examples/apollo4p-evb/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-apollo4p-evb-smoke \
  --system configs/systems/apollo4p-evb.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
