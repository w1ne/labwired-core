#!/bin/sh
# Rebuild dead_sleep.elf from dead_sleep.S (arm-none-eabi-as/ld).
set -e
cd "$(dirname "$0")"
arm-none-eabi-as -o dead_sleep.o dead_sleep.S
arm-none-eabi-ld -nostdlib -e reset --build-id=none \
  -Ttext=0x08000040 --section-start=.vectors=0x08000000 -Tbss=0x20000000 \
  -o dead_sleep.elf dead_sleep.o
arm-none-eabi-strip dead_sleep.elf
rm -f dead_sleep.o
