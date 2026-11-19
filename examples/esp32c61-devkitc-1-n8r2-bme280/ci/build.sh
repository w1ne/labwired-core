#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
CARGO_BUILD_RUSTC_WRAPPER='' cargo build -p firmware-esp32c61-bme280-smoke --release --target riscv32imc-unknown-none-elf

if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  case "$CARGO_TARGET_DIR" in
    /*) TARGET_ROOT="$CARGO_TARGET_DIR" ;;
    *) TARGET_ROOT="$ROOT/$CARGO_TARGET_DIR" ;;
  esac
else
  TARGET_ROOT="$ROOT/target"
fi

mkdir -p examples/esp32c61-devkitc-1-n8r2-bme280/build
cp "$TARGET_ROOT/riscv32imc-unknown-none-elf/release/firmware-esp32c61-bme280-smoke" \
  examples/esp32c61-devkitc-1-n8r2-bme280/build/firmware.elf
