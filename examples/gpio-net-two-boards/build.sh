#!/usr/bin/env bash
# Rebuild the example's firmware (arm-none-eabi-gcc, avr-gcc, clang + lld). The ELFs are
# committed so the tests run without a toolchain.
set -euo pipefail
cd "$(dirname "$0")"
arm-none-eabi-gcc -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -Wl,--build-id=none -T src/stm.ld src/stm.c -o firmware/stm.elf
avr-gcc -mmcu=atmega328p -Os -Wall -Wextra src/avr.c -o firmware/avr.elf
# Contention demo: both boards drive one push-pull wire.
arm-none-eabi-gcc -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -Wl,--build-id=none -T src/stm.ld src/stm_fight.c -o firmware/stm-fight.elf
avr-gcc -mmcu=atmega328p -Os -Wall -Wextra src/avr_fight.c -o firmware/avr-fight.elf
# Demo timing for the browser lab: the same sources, every delay 20x longer.
arm-none-eabi-gcc -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -Wl,--build-id=none -DTIME_SCALE=20u -T src/stm.ld src/stm.c -o firmware/stm-demo.elf
avr-gcc -mmcu=atmega328p -Os -Wall -Wextra -DTIME_SCALE=20 src/avr.c -o firmware/avr-demo.elf
arm-none-eabi-gcc -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -Wl,--build-id=none -DTIME_SCALE=20u -T src/stm.ld src/stm_fight.c -o firmware/stm-fight-demo.elf
# RP2040 and ESP32-C6 peers (env-rp2040.yaml, env-esp32c6.yaml): clang + lld.
clang --target=thumbv6m-none-eabi -mcpu=cortex-m0plus -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -fuse-ld=lld -Wl,--build-id=none -T src/rp2040.ld src/rp2040.c -o firmware/rp2040.elf
clang --target=riscv32-unknown-elf -march=rv32imac -mabi=ilp32 -Os -ffreestanding -fno-builtin -nostdlib \
    -Wall -Wextra -fuse-ld=lld -Wl,--build-id=none -Wl,--no-relax -T src/esp32c6.ld src/esp32c6.c \
    -o firmware/esp32c6.elf
