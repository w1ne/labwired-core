#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

cargo build -p firmware-esp32c3-devkitm-1-n4x-smoke --release \
  --target riscv32imc-unknown-none-elf

mkdir -p examples/esp32c3-devkitm-1-n4x/build
cp target/riscv32imc-unknown-none-elf/release/firmware-esp32c3-devkitm-1-n4x-smoke \
  examples/esp32c3-devkitm-1-n4x/build/firmware.elf
