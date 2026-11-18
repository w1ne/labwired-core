#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT="out/boards/stm32f103c6-low-density"
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f103c6-smoke/io-smoke.yaml \
  --output-dir "$OUT" \
  --no-uart-stdout
