#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-f446re
mkdir -p "$OUT"
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f446re/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
