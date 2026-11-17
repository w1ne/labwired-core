#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
scripts/onboarding_smoke.sh --target-id rpi-pico-2-w-bme280 --crate firmware-pico2w-bme280-smoke --target thumbv8m.main-none-eabi --firmware-build-dir crates/firmware-pico2w-bme280-smoke --script examples/rpi-pico-2-w-bme280/io-smoke.yaml --system configs/systems/rpi-pico-2-w-bme280.yaml --out-dir out/boards/rpi-pico-2-w-bme280/onboarding-smoke
