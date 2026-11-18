#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT_DIR="${ONBOARDING_EVIDENCE_DIR:-out/boards/nucleo-h533re}"
scripts/onboarding_smoke.sh \
  --target-id nucleo-h533re \
  --script examples/nucleo-h533re/io-smoke.yaml \
  --system configs/systems/nucleo-h533re.yaml \
  --crate firmware-nucleo-h533re-smoke \
  --target thumbv8m.main-none-eabi \
  --out-dir "$OUT_DIR"
