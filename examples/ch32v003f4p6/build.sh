#!/bin/sh
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$example_dir/../.." && pwd)
output="$repo_root/tests/fixtures/ch32v003f4p6-blink.elf"

if [ -n "${RISCV_CC:-}" ]; then
    cc=$RISCV_CC
elif command -v riscv-none-elf-gcc >/dev/null 2>&1; then
    cc=riscv-none-elf-gcc
elif command -v riscv64-unknown-elf-gcc >/dev/null 2>&1; then
    cc=riscv64-unknown-elf-gcc
elif command -v riscv32-esp-elf-gcc >/dev/null 2>&1; then
    cc=riscv32-esp-elf-gcc
else
    echo "RISC-V GCC with RV32E/ILP32E support not found; set RISCV_CC" >&2
    exit 127
fi

mkdir -p "$(dirname -- "$output")"
"$cc" \
    -march=rv32e -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -nostdlib -nostartfiles \
    -Wl,-T,"$example_dir/link.x" -Wl,--no-relax \
    -o "$output" \
    "$example_dir/start.S" "$example_dir/main.c"

printf 'Built %s\n' "$output"
