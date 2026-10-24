#!/bin/sh
# Rebuild tick_wait.elf from tick_wait.S (arm-none-eabi-as/ld).
set -e
cd "$(dirname "$0")"
arm-none-eabi-as -o tick_wait.o tick_wait.S
arm-none-eabi-ld -nostdlib -e reset --build-id=none \
  -Ttext=0x08000040 --section-start=.vectors=0x08000000 -Tbss=0x20000000 \
  -o tick_wait.elf tick_wait.o
arm-none-eabi-strip tick_wait.elf
rm -f tick_wait.o
