#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo test -q -p labwired-core --test avr_atmega328pb_usart1
cargo run -q -p labwired-cli -- test \
  --script examples/atmega328pb-usart1/io-smoke.yaml \
  --output-dir out/boards/atmega328pb-usart1/io-smoke \
  --no-uart-stdout
