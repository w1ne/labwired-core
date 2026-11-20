#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
scripts/onboarding_smoke.sh \
  --target-id pico-w-bme280 \
  --script examples/pico-w-bme280/io-smoke.yaml \
  --system configs/systems/pico-w-bme280.yaml \
  --firmware-build-script examples/pico-w-bme280/ci/build.sh \
  --out-dir examples/pico-w-bme280/evidence/run
