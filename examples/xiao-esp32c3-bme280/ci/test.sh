#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
examples/xiao-esp32c3-bme280/verify-fixture.sh
out="out/boards/seeed-xiao-esp32c3-bme280"
cargo run -q -p labwired-cli -- test \
  --script examples/xiao-esp32c3-bme280/bme280-smoke.yaml \
  --output-dir "${out}" \
  --no-uart-stdout
