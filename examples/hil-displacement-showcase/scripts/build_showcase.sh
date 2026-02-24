#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CORE_DIR="$(cd "$EXAMPLE_DIR/../.." && pwd)"

cd "$CORE_DIR"

echo "==> Building HIL Displacement Showcase Firmware"
cargo build -p firmware-hil-showcase --release --target thumbv7m-none-eabi

echo "==> Converting to binary"
cargo objcopy -p firmware-hil-showcase --release --target thumbv7m-none-eabi -- -O binary \
  examples/hil-displacement-showcase/hil_displacement_showcase.bin

echo "==> Success: examples/hil-displacement-showcase/hil_displacement_showcase.bin"
