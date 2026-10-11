#!/usr/bin/env bash
# Rebuild the exti-once fixture ELFs (clang + ld.lld). The ELFs are committed
# so the test runs without a toolchain.
set -euo pipefail
cd "$(dirname "$0")"
ARM=(clang -Os -ffreestanding -fno-builtin -nostdlib -fuse-ld=lld -Wall -Wextra
     -Xlinker --build-id=none -Xlinker -z -Xlinker max-page-size=16 -T exti_once.ld)
"${ARM[@]}" --target=thumbv7m-none-eabi -mcpu=cortex-m3 -DCHIP_F1 exti_once.c -o exti-once-f1.elf
"${ARM[@]}" --target=thumbv7em-none-eabi -mcpu=cortex-m4 -DCHIP_F4 exti_once.c -o exti-once-f4.elf
"${ARM[@]}" --target=thumbv6m-none-eabi -mcpu=cortex-m0plus -DCHIP_G0 exti_once.c -o exti-once-g0.elf
"${ARM[@]}" --target=thumbv6m-none-eabi -mcpu=cortex-m0plus -DCHIP_L0 exti_once.c -o exti-once-l0.elf
"${ARM[@]}" --target=thumbv8m.main-none-eabi -mcpu=cortex-m33 -DCHIP_U5 exti_once.c -o exti-once-u5.elf
