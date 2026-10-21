#!/usr/bin/env bash
# atmega2560's CLI proof: runs the committed Arduino-core Mega sketch (built for
# arduino:avr:mega) and asserts its banner plus a report line with A8/A9 read
# through ADCSRB.MUX5 and a value returned through an EICALL.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/arduino-mega-2560-io
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-mega-2560-io/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
