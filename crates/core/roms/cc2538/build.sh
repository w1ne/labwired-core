#!/usr/bin/env bash
# Rebuild the CC2538 images compiled into labwired-core, and commit them:
# - cc2538_rom.bin from cc2538_rom.S (arm-none-eabi binutils);
# - cc2538_info_page.bin: the head of the flash information page
#   (0x0028_0000): erased 0xFF up to the primary IEEE address at +0x28, then
#   00:12:4B:00:00:00:00:01 (TI OUI + a fixed device part) in the byte order
#   TI parts ship with (Contiki-NG ieee-addr.c: "00 4B 12 00 04 03 02 01"
#   for 00:12:4B:00:01:02:03:04). The rest of the page reads erased.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-as -mcpu=cortex-m3 -mthumb -o "$tmp/rom.o" cc2538_rom.S
arm-none-eabi-ld -Ttext=0x0 -e _rom_start -o "$tmp/rom.elf" "$tmp/rom.o"
arm-none-eabi-objcopy -O binary -j .text "$tmp/rom.elf" cc2538_rom.bin
{
  head -c 40 /dev/zero | tr '\0' '\377'
  printf '\x00\x4b\x12\x00\x01\x00\x00\x00'
} > cc2538_info_page.bin
sha256sum cc2538_rom.bin cc2538_info_page.bin
