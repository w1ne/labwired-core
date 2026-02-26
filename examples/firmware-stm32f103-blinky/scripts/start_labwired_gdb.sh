#!/usr/bin/env bash
set -euo pipefail

# Start LabWired GDB server for firmware-stm32f103-blinky on localhost:3333.
# Run from anywhere.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BLINKY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CORE_DIR="$(cd "${BLINKY_DIR}/../.." && pwd)"

pushd "${CORE_DIR}" >/dev/null

cargo build -p firmware-stm32f103-blinky --target thumbv7m-none-eabi
cargo build -p labwired-cli

exec cargo run -q -p labwired-cli -- \
  --gdb 3333 \
  --firmware "${CORE_DIR}/target/thumbv7m-none-eabi/debug/firmware-stm32f103-blinky" \
  --system "${BLINKY_DIR}/system.yaml" \
  --max-steps 2000000
