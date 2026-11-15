#!/usr/bin/env bash
# Run the committed ESP32-S2 ELF against its descriptor-defined UART0 model.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/esp32s2-devkitc-1-n8r2
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitc-1-n8r2/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
