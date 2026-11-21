#!/bin/sh
# Compile the pinned firmware/pomodoro.c, unchanged, as portable Flex firmware.
# CC is a C compiler with a wasm32 target and its linker: clang with wasm-ld,
# or `zig cc`. The committed firmware.wasm is from `zig cc` of Zig 0.17.1:
#   CC="uv run --no-project --with ziglang==0.17.1 python -m ziglang cc" sh build.sh
set -eu
cd "$(dirname "$0")"
${CC:-clang} --target=wasm32-freestanding -nostdlib -ffreestanding \
  -Os -Wall -Wno-ignored-attributes -Wno-unknown-attributes \
  -Ishim -Ifirmware \
  -Wl,--no-entry -Wl,--export=__main_void -Wl,--strip-all \
  -o firmware.wasm firmware/pomodoro.c shim/flex_ch32fun.c
