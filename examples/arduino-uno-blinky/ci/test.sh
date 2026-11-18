#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-blinky/io-smoke.yaml \
  --output-dir out/boards/arduino-uno-r3-existing/blinky \
  --no-uart-stdout

cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-io/io-smoke.yaml \
  --output-dir out/boards/arduino-uno-r3-existing/io \
  --no-uart-stdout
