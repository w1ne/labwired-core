#!/usr/bin/env bash
# Rebuild bootrom.bin (compiled into labwired-core) from rp2040_bootrom.S
# with arm-none-eabi binutils, and commit it. Padded to the 16 KiB ROM.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-as -mcpu=cortex-m0plus -mthumb -o "$tmp/rom.o" rp2040_bootrom.S
arm-none-eabi-ld -Ttext=0x00000000 -e _rom_start -o "$tmp/rom.elf" "$tmp/rom.o"
arm-none-eabi-objcopy -O binary -j .text --pad-to 0x4000 --gap-fill 0 "$tmp/rom.elf" bootrom.bin
sha256sum bootrom.bin 2>/dev/null || shasum -a 256 bootrom.bin
