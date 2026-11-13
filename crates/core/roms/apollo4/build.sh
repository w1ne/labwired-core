#!/usr/bin/env bash
# Rebuild apollo4_bootrom.bin (compiled into labwired-core) from
# apollo4_bootrom.S with arm-none-eabi binutils, and commit it.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-as -mcpu=cortex-m4 -mthumb -o "$tmp/rom.o" apollo4_bootrom.S
arm-none-eabi-ld -Ttext=0x08000000 -e _rom_start -o "$tmp/rom.elf" "$tmp/rom.o"
arm-none-eabi-objcopy -O binary -j .text "$tmp/rom.elf" apollo4_bootrom.bin
sha256sum apollo4_bootrom.bin
