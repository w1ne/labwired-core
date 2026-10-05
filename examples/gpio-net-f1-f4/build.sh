#!/usr/bin/env bash
# Rebuild the example's firmware (clang + ld.lld for the two STM32s, avr-gcc
# for the ATmega328P). The ELFs are committed so the tests run without a
# toolchain.
set -euo pipefail
cd "$(dirname "$0")"
ARM=(clang -Os -ffreestanding -fno-builtin -nostdlib -fuse-ld=lld -Wall -Wextra
     -Xlinker --build-id=none -Xlinker -z -Xlinker max-page-size=16)
"${ARM[@]}" --target=thumbv7m-none-eabi -mcpu=cortex-m3 -T src/f103.ld src/f103.c -o firmware/f103.elf
"${ARM[@]}" --target=thumbv7em-none-eabi -mcpu=cortex-m4 -T src/f401.ld src/f401.c -o firmware/f401.elf
avr-gcc -mmcu=atmega328p -Os -Wall -Wextra src/avr.c -o firmware/avr.elf
