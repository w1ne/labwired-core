#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
BUILD_DIR="${REPO_ROOT}/examples/esp32c3-devkitm-1-n4x-bme280/firmware/.pio/build/devkitm1_n4x"
pio run --project-dir "${REPO_ROOT}/examples/esp32c3-devkitm-1-n4x-bme280/firmware" \
  --environment devkitm1_n4x
pio pkg exec -p tool-esptool -- esptool --chip esp32c3 merge-bin \
  --flash-mode dio --flash-freq 80m --flash-size 4MB \
  --output "${BUILD_DIR}/merged.bin" \
  0x0 "${BUILD_DIR}/bootloader.bin" \
  0x8000 "${BUILD_DIR}/partitions.bin" \
  0x10000 "${BUILD_DIR}/firmware.bin"
