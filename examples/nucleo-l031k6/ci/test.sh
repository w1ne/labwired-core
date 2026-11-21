#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-l031k6
mkdir -p "$OUT"
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-l031k6/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
