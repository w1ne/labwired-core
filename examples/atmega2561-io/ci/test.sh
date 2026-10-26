#!/usr/bin/env bash
# atmega2561 CLI smoke: the committed 3-byte-PC AVR image prints "2561 OK"
# through USART0. The Rust integration test separately asserts the PB5 output.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/atmega2561-io
cargo run -q -p labwired-cli -- test \
  --script examples/atmega2561-io/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
