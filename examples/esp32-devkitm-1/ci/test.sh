#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
examples/esp32-devkitm-1/verify-bme280-fixture.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32-devkitm-1/bme280-smoke.yaml \
  --output-dir out/boards/esp32-devkitm-1/bme280-smoke \
  --no-uart-stdout
