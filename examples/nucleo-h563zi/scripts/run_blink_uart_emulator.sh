#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CORE_DIR="$(cd "$EXAMPLE_DIR/../.." && pwd)"

run() {
  echo
  echo "==> $*"
  "$@"
}

cd "$CORE_DIR"

echo "Running NUCLEO-H563ZI blink+UART check in emulator"
run cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
run cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml

echo

echo "Emulator blink+UART check passed."
