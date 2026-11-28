#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
scripts/onboarding_smoke.sh \
  --target-id stm32f103r6-low-density \
  --crate firmware-stm32f103-lowdensity-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103r6-smoke/io-smoke.yaml \
  --system examples/stm32f103r6-smoke/system.yaml \
  --out-dir examples/stm32f103r6-smoke/evidence
