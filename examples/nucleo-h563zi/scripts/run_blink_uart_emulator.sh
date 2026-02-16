#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CORE_DIR="$(cd "$EXAMPLE_DIR/../.." && pwd)"
ARTIFACT_DIR="$(mktemp -d /tmp/h563_emu_uart.XXXXXX)"

run() {
  echo
  echo "==> $*"
  "$@"
}

cleanup() {
  rm -rf "$ARTIFACT_DIR"
}
trap cleanup EXIT

cd "$CORE_DIR"

echo "Running NUCLEO-H563ZI blink+UART check in emulator"
run cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
run cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml --no-uart-stdout --output-dir "$ARTIFACT_DIR"

echo
echo "==> Emulator UART output"
cat "$ARTIFACT_DIR/uart.log"

grep -q "H563-IO" "$ARTIFACT_DIR/uart.log"
grep -q "PB0=1 PF4=1 PG4=1" "$ARTIFACT_DIR/uart.log"
grep -q "PB0=0 PF4=0 PG4=0" "$ARTIFACT_DIR/uart.log"

echo

echo "Emulator blink+UART check passed."
