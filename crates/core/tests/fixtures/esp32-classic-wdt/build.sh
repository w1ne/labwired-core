#!/usr/bin/env bash
# Rebuild the classic-ESP32 MWDT fixtures. Needs an Espressif Xtensa GCC
# (PlatformIO package `toolchain-xtensa-esp-elf`, or ESP-IDF's
# `xtensa-esp-elf-gcc`).
set -euo pipefail
cd "$(dirname "$0")"
CC="${CC:-xtensa-esp32-elf-gcc}"
# The esp32-prefixed driver already selects the LX6 multilib; no -mcpu needed.
FLAGS=(-nostdlib -nostartfiles -mtext-section-literals
       '-Wl,-T,wdt.ld' '-Wl,--build-id=none')
"$CC" "${FLAGS[@]}" -o wdt_starve.elf wdt_starve.S
"$CC" "${FLAGS[@]}" -o wdt_feed.elf wdt_feed.S
