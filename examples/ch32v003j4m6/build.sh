#!/bin/sh
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$example_dir/../.." && pwd)
shared_dir="$repo_root/examples/ch32v003f4p6"
output="$repo_root/tests/fixtures/ch32v003j4m6-blink-pc4.elf"

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

# Same start-up code and linker script as the F4P6 example: the die and
# memory map are identical, only the package differs.
mkdir -p "$(dirname -- "$output")"
"$cc" \
    -march=rv32ec -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -nostdlib -nostartfiles \
    -Wl,-T,"$shared_dir/link.x" -Wl,--no-relax \
    -o "$output" \
    "$shared_dir/start.S" "$example_dir/main.c"

printf 'Built %s\n' "$output"
