#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"
cargo build -p firmware-esp32c6-devkitc-bme280-smoke \
  --release --target riscv32imc-unknown-none-elf
