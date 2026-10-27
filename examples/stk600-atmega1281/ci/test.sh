#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo test -q -p labwired-core --test avr_stk600_atmega1281_firmware
cargo run -q -p labwired-cli -- test \
  --script examples/stk600-atmega1281/io-smoke.yaml \
  --output-dir out/boards/stk600-atmega1281/io-smoke \
  --no-uart-stdout
