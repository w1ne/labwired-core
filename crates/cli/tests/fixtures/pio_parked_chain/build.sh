#!/bin/sh
# Rebuild pio_park.elf from pio_park.S (arm-none-eabi-as/ld).
set -e
cd "$(dirname "$0")"
arm-none-eabi-as -o pio_park.o pio_park.S
arm-none-eabi-ld -nostdlib -e reset --build-id=none \
  -Ttext=0x10000100 --section-start=.vectors=0x10000000 -Tbss=0x20000000 \
  -o pio_park.elf pio_park.o
arm-none-eabi-strip pio_park.elf
rm -f pio_park.o
