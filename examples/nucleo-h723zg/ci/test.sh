#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT_DIR="${ONBOARDING_EVIDENCE_DIR:-out/boards/nucleo-h723zg}"
scripts/onboarding_smoke.sh \
  --target-id nucleo-h723zg \
  --script examples/nucleo-h723zg/io-smoke.yaml \
  --system configs/systems/nucleo-h723zg.yaml \
  --crate firmware-nucleo-h723zg-smoke \
  --target thumbv7em-none-eabi \
  --out-dir "$OUT_DIR"
# This fixture includes the stock PlatformIO STM32Cube CMSIS SystemInit path,
# which accesses FLASH_ACR and the FMC bank-disable register before main.
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-h723zg/stm32cube-smoke.yaml \
  --output-dir "$OUT_DIR/stm32cube" \
  --no-uart-stdout
