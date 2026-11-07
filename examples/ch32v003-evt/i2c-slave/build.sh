#!/usr/bin/env bash
# Rebuild the STM32F103 controller firmware with arm-none-eabi-gcc (no vendor
# SDK). The ELFs are committed so the tests run without a toolchain. The
# CH32V003 side is ch32fun's i2c_slave, built by ../build-ch32fun-examples.sh.
set -euo pipefail
cd "$(dirname "$0")"
CC=${CC:-arm-none-eabi-gcc}
F1=(-mthumb -mcpu=cortex-m3 -Os -ffreestanding -fno-builtin -nostdlib -Wall -Wextra
    -Wl,--build-id=none -T ../../gpio-net-buses/src/f1.ld)
mkdir -p firmware
"$CC" "${F1[@]}" src/f1_controller.c -o firmware/f1-i2c-controller.elf
"$CC" "${F1[@]}" -DPAST_END=1 src/f1_controller.c -o firmware/f1-i2c-controller-past-end.elf
chmod 0644 firmware/*.elf
