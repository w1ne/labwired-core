#!/usr/bin/env bash
# Rebuild ../rp2350-dcp-canned.elf from harness.S + the vendored pico-sdk
# DCP sequence files, with arm-none-eabi binutils, and commit it.
set -euo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
arm-none-eabi-gcc -x assembler-with-cpp -mcpu=cortex-m33 -mthumb -I. -c harness.S -o "$tmp/h.o"
arm-none-eabi-ld -Ttext=0x10000100 --section-start=.vectors=0x10000000 -e t_park -o ../rp2350-dcp-canned.elf "$tmp/h.o"
sha256sum ../rp2350-dcp-canned.elf 2>/dev/null || shasum -a 256 ../rp2350-dcp-canned.elf
