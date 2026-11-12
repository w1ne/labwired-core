#!/usr/bin/env bash
# Runs the crate-built BME280 smoke against the narrow NUCLEO-F103RB
# descriptor and asserts the committed onboarding markers.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-f103rb-bme280
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f103rb-bme280/onboarding-smoke.yaml \
  --output-dir "$OUT/onboarding" \
  --no-uart-stdout
