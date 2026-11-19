#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
scripts/onboarding_smoke.sh \
  --target-id pico2-bme280 \
  --crate firmware-pico2-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --firmware-build-dir crates/firmware-pico2-bme280-smoke \
  --script examples/pico2-bme280/io-smoke.yaml \
  --system configs/systems/pico2-bme280.yaml \
  --out-dir out/boards/pico2-bme280/onboarding-smoke
