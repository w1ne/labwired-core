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

echo "Running NUCLEO-H563ZI emulator capability showcase"

run cargo test -p labwired-core test_flash_boot_alias_read_and_write -- --nocapture

run cargo build -p firmware-h563-demo --release --target thumbv7m-none-eabi
run cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
run cargo build -p firmware-h563-fullchip-demo --release --target thumbv7m-none-eabi

run cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/uart-smoke.yaml
run cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml
run cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/fullchip-smoke.yaml

echo

echo "NUCLEO-H563ZI emulator showcase completed successfully."
