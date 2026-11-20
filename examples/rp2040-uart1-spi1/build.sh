#!/usr/bin/env bash
# Rebuild the smoke firmware (arm-none-eabi-gcc). The ELF is committed so the
# smoke runs without a toolchain.
set -euo pipefail
cd "$(dirname "$0")"
arm-none-eabi-gcc -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -Wl,--build-id=none -T src/rp2040.ld src/main.c -o firmware/uart1-spi1.elf
