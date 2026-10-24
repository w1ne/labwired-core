#!/bin/sh
# Rebuild blink.elf from blink.S with the esp32 GNU toolchain (PlatformIO's
# toolchain-xtensa-esp32 or ESP-IDF's xtensa-esp32-elf).
set -e
cd "$(dirname "$0")"
T=${XTENSA_ESP32_BIN:-$HOME/.platformio/packages/toolchain-xtensa-esp32/bin}
"$T/xtensa-esp32-elf-as" --no-transform -o blink.o blink.S
"$T/xtensa-esp32-elf-ld" -Ttext=0x40080000 -n -e _start --build-id=none -o blink.elf blink.o
"$T/xtensa-esp32-elf-strip" blink.elf
rm -f blink.o
