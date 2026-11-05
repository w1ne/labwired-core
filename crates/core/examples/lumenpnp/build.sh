#!/usr/bin/env bash
set -eu
if [ -n "${ARM_TOOLCHAIN_BIN:-}" ]; then
  export PATH="$ARM_TOOLCHAIN_BIN:$PATH"
fi
example_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
arm_cc=${ARM_CC:-arm-none-eabi-gcc}
arm_size=${ARM_SIZE:-arm-none-eabi-size}
arm_nm=${ARM_NM:-arm-none-eabi-nm}
cd "$example_dir"
"$arm_cc" -mcpu=cortex-m4 -mthumb -mfloat-abi=soft \
  -O2 -g3 -ffunction-sections -fdata-sections -ffreestanding -fno-builtin \
  -ffile-prefix-map="$example_dir"=. -Wa,--debug-prefix-map="$example_dir"=. \
  -nostdlib -Wl,--gc-sections -Wl,-Map=firmware.map \
  -T linker.ld startup.S firmware.c \
  -lgcc -o firmware.elf
"$arm_size" "$example_dir/firmware.elf"
"$arm_nm" -n "$example_dir/firmware.elf" > "$example_dir/symbols.txt"
