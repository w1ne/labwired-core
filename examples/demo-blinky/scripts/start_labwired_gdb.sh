#!/usr/bin/env bash
set -euo pipefail

# Start LabWired GDB server for demo-blinky on localhost:3333.
# Run from anywhere.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BLINKY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CORE_DIR="$(cd "${BLINKY_DIR}/../.." && pwd)"

pushd "${CORE_DIR}" >/dev/null

cargo build -p demo-blinky --target thumbv7m-none-eabi
cargo build -p labwired-cli

exec cargo run -q -p labwired-cli -- \
  --gdb 3333 \
  --firmware "${CORE_DIR}/target/thumbv7m-none-eabi/debug/demo-blinky" \
  --system "${BLINKY_DIR}/system.yaml" \
  --max-steps 2000000
