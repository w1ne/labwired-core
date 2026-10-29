#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo test -q -p labwired-core --test avr_atmega1281 --test avr_atmega1281_firmware
cargo run -q -p labwired-cli -- test \
  --script examples/atmega1281-io/io-smoke.yaml \
  --output-dir out/boards/atmega1281-io/io-smoke \
  --no-uart-stdout
