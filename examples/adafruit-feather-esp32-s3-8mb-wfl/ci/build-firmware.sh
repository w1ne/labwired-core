#!/usr/bin/env bash
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
FIRMWARE_DIR="${REPO_ROOT}/target/xtensa-esp32s3-none-elf/release"

mkdir -p "${FIRMWARE_DIR}"
pio pkg exec -p toolchain-xtensa-esp-elf -- xtensa-esp32s3-elf-gcc \
  -mlongcalls \
  -mtext-section-literals \
  -nostdlib \
  -nostartfiles \
  -fno-stack-protector \
  -fno-unwind-tables \
  -fno-asynchronous-unwind-tables \
  -Wl,--build-id=none \
  -Wl,-T,"${EXAMPLE_DIR}/firmware/link.ld" \
  -Wl,-e,_start \
  -o "${FIRMWARE_DIR}/firmware-adafruit-feather-esp32-s3-8mb-wfl-smoke" \
  "${EXAMPLE_DIR}/firmware/max17048_smoke.c"
