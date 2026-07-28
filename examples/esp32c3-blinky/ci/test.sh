#!/usr/bin/env bash
# Runs the committed ESP32-C3 blinky ELF and asserts the UART blink log plus the
# GPIO_ENABLE readback. This example shipped ungated and its max_steps budget
# silently stopped matching its own assertions; this script keeps that from
# recurring.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/esp32c3-blinky
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c3-blinky/test-blink.yaml \
  --output-dir "$OUT/blink" \
  --no-uart-stdout
