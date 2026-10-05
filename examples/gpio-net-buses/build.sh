#!/usr/bin/env bash
# Rebuild the example's firmware with clang + lld (no vendor SDK). The ELFs are
# committed so the tests run without a toolchain.
set -euo pipefail
cd "$(dirname "$0")"
CC=${CC:-clang}
FLAGS=(--target=thumbv6m-none-eabi -mcpu=cortex-m0plus -mthumb -Os -ffreestanding
       -fno-builtin -nostdlib -Wall -Wextra -fuse-ld=lld -Wl,--build-id=none -Wl,-z,max-page-size=4096 -T src/g0.ld)
for fw in spi_master spi_slave i2c_controller i2c_target; do
    "$CC" "${FLAGS[@]}" "src/$fw.c" -o "firmware/${fw//_/-}.elf"
done
