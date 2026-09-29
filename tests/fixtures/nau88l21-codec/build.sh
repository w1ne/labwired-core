#!/bin/sh
# Rebuild the committed codec fixture ELF. Needs arm-none-eabi binutils.
set -eu
cd "$(dirname "$0")"
arm-none-eabi-as -mcpu=cortex-m7 -mthumb -o codec.o codec.S
arm-none-eabi-ld -T link.ld -o ../imxrt-lpi2c-nau88l21.elf codec.o
rm -f codec.o
