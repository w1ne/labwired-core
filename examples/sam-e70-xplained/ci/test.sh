#!/usr/bin/env bash
# SAM E70 Xplained CLI proof: boot the SAM E70 smoke, assert the USART1/LED/
# button stream, then require the firmware's instruction stream to be fully
# supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/sam-e70-xplained
cargo run -q -p labwired-cli -- test \
  --script examples/sam-e70-xplained/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-sam-e70-xplained-smoke \
  --system configs/systems/sam-e70-xplained.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
