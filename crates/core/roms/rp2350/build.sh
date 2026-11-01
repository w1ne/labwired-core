#!/usr/bin/env bash
# Rebuild rp2350_bootrom.bin (compiled into labwired-core) from
# rp2350_bootrom.S with arm-none-eabi binutils, and commit it.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-as -mcpu=cortex-m33 -mthumb -o "$tmp/rom.o" rp2350_bootrom.S
arm-none-eabi-ld -Ttext=0x00000000 -e _rom_start -o "$tmp/rom.elf" "$tmp/rom.o"
arm-none-eabi-objcopy -O binary -j .text "$tmp/rom.elf" rp2350_bootrom.bin
shasum -a 256 rp2350_bootrom.bin
