#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-nucleo-h533re-smoke --release --target thumbv8m.main-none-eabi
cp target/thumbv8m.main-none-eabi/release/firmware-nucleo-h533re-smoke \
  tests/fixtures/stm32h533re-smoke.elf
