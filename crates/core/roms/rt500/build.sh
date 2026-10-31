#!/usr/bin/env bash
# Rebuild rt500_rom.bin (compiled into labwired-core) from rt500_rom.S with
# arm-none-eabi binutils, and commit it.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-as -mcpu=cortex-m33 -mthumb -o "$tmp/rom.o" rt500_rom.S
arm-none-eabi-ld -Ttext=0x03000000 -e _rom_start -o "$tmp/rom.elf" "$tmp/rom.o"
arm-none-eabi-objcopy -O binary -j .text "$tmp/rom.elf" rt500_rom.bin
sha256sum rt500_rom.bin
