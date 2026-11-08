#!/usr/bin/env bash
set -euo pipefail

cargo build \
  -p firmware-esp32c3-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --release \
  --features xiao
