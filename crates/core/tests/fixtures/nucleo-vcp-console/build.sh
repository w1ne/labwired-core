#!/usr/bin/env bash
# Rebuild the nucleo-vcp-console fixture ELFs (arm-none-eabi-gcc).
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck disable=SC2054  # -Wl,--build-id=none is one argument
CF=(-mthumb -Os -ffreestanding -fno-builtin -nostdlib -Wall -Wextra
    -Wl,--build-id=none -T vcp-console.ld)
arm-none-eabi-gcc -mcpu=cortex-m3 "${CF[@]}" -DFAMILY_F1 vcp-console.c -o vcp-console-f103.elf
arm-none-eabi-gcc -mcpu=cortex-m4 "${CF[@]}" vcp-console.c -o vcp-console-f4.elf
