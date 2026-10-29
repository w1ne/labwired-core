#!/usr/bin/env bash
# NUCLEO-L552ZE-Q CLI proof: boot the L552 smoke, assert the LPUART1/LED/button
# stream, then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-l552ze-q
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-l552ze-q/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
