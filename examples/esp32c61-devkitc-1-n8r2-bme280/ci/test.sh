#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc-1-n8r2-bme280 \
  --firmware-build-script examples/esp32c61-devkitc-1-n8r2-bme280/ci/build.sh \
  --script examples/esp32c61-devkitc-1-n8r2-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32c61-devkitc-1-n8r2-bme280.yaml \
  --out-dir examples/esp32c61-devkitc-1-n8r2-bme280/evidence/run
