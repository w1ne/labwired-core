#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/stm32f103-blue-pill-bme280
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f103-blue-pill-bme280/io-smoke.yaml \
  --output-dir "$OUT/onboarding" \
  --no-uart-stdout
