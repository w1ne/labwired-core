#!/bin/sh
# Rebuild pio_stall.elf from pio_stall.S with clang + rust-lld
# (any thumbv6m-capable clang and a GNU-flavour lld work).
set -e
cd "$(dirname "$0")"
LLD=${LLD:-$(ls "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld | head -1)}
clang --target=thumbv6m-none-eabi -c pio_stall.S -o pio_stall.o
"$LLD" -flavor gnu -nostdlib -e reset --build-id=none \
  -N -Ttext=0x10000100 --section-start=.vectors=0x10000000 -Tbss=0x20000000 \
  -o pio_stall.elf pio_stall.o
rm -f pio_stall.o
