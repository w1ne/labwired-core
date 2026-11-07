#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE_DIR="${REPO_ROOT}/examples/esp32s2-devkitm1-n4r2"
BUILD_DIR="${REPO_ROOT}/target/esp32s2-devkitm1-n4r2"
CC="${XTENSA_ESP32S2_CC:-xtensa-esp32s2-elf-gcc}"

if ! command -v "${CC}" >/dev/null 2>&1; then
  echo "Missing ESP32-S2 cross compiler: ${CC}" >&2
  echo "Set XTENSA_ESP32S2_CC to Espressif's xtensa-esp32s2-elf-gcc." >&2
  exit 127
fi

mkdir -p "${BUILD_DIR}"
"${CC}" \
  -mlongcalls -mtext-section-literals \
  -ffreestanding -fno-builtin -fno-pic \
  -O2 -Wall -Wextra -Werror \
  -nostdlib -nostartfiles \
  -Wl,--build-id=none -Wl,-e,_start \
  -Wl,-T,"${EXAMPLE_DIR}/firmware/linker.ld" \
  -Wl,-Map,"${BUILD_DIR}/firmware.map" \
  -o "${BUILD_DIR}/firmware.elf" \
  "${EXAMPLE_DIR}/firmware/main.c"

echo "Built ${BUILD_DIR}/firmware.elf"
