#!/usr/bin/env bash
# Rebuild console.elf. Needs the Espressif Xtensa GCC (PlatformIO package
# toolchain-xtensa-esp-elf, or ESP-IDF's xtensa-esp-elf).
set -euo pipefail
cd "$(dirname "$0")"
CC="${CC:-xtensa-esp32-elf-gcc}"
"$CC" -nostdlib -nostartfiles -mtext-section-literals -Wl,-T,console.ld \
  -Wl,--build-id=none -o console.elf console.S
