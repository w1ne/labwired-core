#!/usr/bin/env bash
# atmega2561's CLI proof: the committed 3-byte-PC AVR image prints "2561 OK"
# through USART0 and drives PB5, which io-smoke.yaml asserts.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/atmega2561-io
cargo run -q -p labwired-cli -- test \
  --script examples/atmega2561-io/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
