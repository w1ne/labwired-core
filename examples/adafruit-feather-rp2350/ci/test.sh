#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-rp2350 \
  --crate firmware-adafruit-feather-rp2350-smoke \
  --target thumbv8m.main-none-eabi \
  --firmware-build-dir crates/firmware-adafruit-feather-rp2350-smoke \
  --script examples/adafruit-feather-rp2350/io-smoke.yaml \
  --system configs/systems/adafruit-feather-rp2350.yaml \
  --out-dir out/boards/adafruit-feather-rp2350/onboarding-smoke
