#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT_DIR="${ONBOARDING_EVIDENCE_DIR:-out/boards/nucleo-h723zg}"
scripts/onboarding_smoke.sh \
  --target-id nucleo-h723zg \
  --script examples/nucleo-h723zg/io-smoke.yaml \
  --system configs/systems/nucleo-h723zg.yaml \
  --crate firmware-nucleo-h723zg-smoke \
  --target thumbv7em-none-eabi \
  --out-dir "$OUT_DIR"
