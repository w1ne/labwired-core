#!/usr/bin/env bash
# Runs the committed classic-ESP32 TIER1 ELF against the DevKitC-32E UART1
# smoke chip descriptor and asserts the documented TIER1 UART markers.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/esp32-devkitc-32e-uart1
cargo run -q -p labwired-cli -- test \
  --script examples/esp32-devkitc-32e-uart1/tier1-smoke.yaml \
  --output-dir "$OUT/tier1" \
  --no-uart-stdout
