#!/usr/bin/env bash
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
FIRMWARE_DIR="${REPO_ROOT}/target/xtensa-esp32s3-none-elf/release"

# Build the copy-paste Arduino example as well as the small fast-boot image the
# repository simulator uses for the exact board-level I2C smoke.
pio run --project-dir "${EXAMPLE_DIR}" --environment nano_esp32

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
  -o "${FIRMWARE_DIR}/firmware-arduino-nano-esp32-bme280-smoke" \
  "${EXAMPLE_DIR}/firmware/bme280_smoke.c"
