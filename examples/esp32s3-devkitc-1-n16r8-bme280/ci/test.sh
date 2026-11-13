#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
examples/esp32s3-devkitc-1-n16r8-bme280/verify-fixture.sh
out="out/boards/esp32s3-devkitc-1-n16r8-bme280"
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s3-devkitc-1-n16r8-bme280/bme280-smoke.yaml \
  --output-dir "${out}" \
  --no-uart-stdout
