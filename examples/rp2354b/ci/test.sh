#!/usr/bin/env bash
# Runs the committed RP2354B smoke ELF against configs/systems/rp2354b.yaml
# and asserts the UART0 banner and the GPIO47 edges.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/rp2354b
cargo run -q -p labwired-cli -- test \
  --script examples/rp2354b/io-smoke.yaml \
  --output-dir "$OUT/smoke" \
  --no-uart-stdout
