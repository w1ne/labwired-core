#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"
examples/arduino-nano-esp32-bme280/verify-fixture.sh
out="out/boards/arduino-nano-esp32-bme280"
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-esp32-bme280/bme280-smoke.yaml \
  --output-dir "${out}" \
  --no-uart-stdout
