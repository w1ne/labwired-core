#!/usr/bin/env bash
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"

pio run --project-dir "${EXAMPLE_DIR}" --environment nano_esp32

FIRMWARE_DIR="${REPO_ROOT}/target/xtensa-esp32s3-none-elf/release"
mkdir -p "${FIRMWARE_DIR}"
rm -f "${FIRMWARE_DIR}/partitions.bin"

# The board-specific smoke is a small bare-metal image. The simulator's
# fast-boot path can execute this GPIO slice directly; Arduino's full ESP-IDF
# startup needs the faithful mask-ROM image path and must not be mistaken for
# a GPIO-model failure.
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
  -o "${FIRMWARE_DIR}/firmware-arduino-nano-esp32-smoke" \
  "${EXAMPLE_DIR}/firmware/board_gpio_smoke.c"
