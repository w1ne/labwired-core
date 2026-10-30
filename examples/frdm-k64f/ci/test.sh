#!/usr/bin/env bash
# FRDM-K64F CLI proof: boot the UART-only smoke, assert the UART0/LED stream,
# then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/frdm-k64f
cargo run -q -p labwired-cli -- test \
  --script examples/frdm-k64f/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
# Stock Zephyr v3.7.0 hello_world (committed fixture, no Zephyr build needed).
cargo run -q -p labwired-cli -- test \
  --script examples/frdm-k64f/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
