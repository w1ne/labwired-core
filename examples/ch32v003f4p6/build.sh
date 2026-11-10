#!/bin/sh
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$example_dir/../.." && pwd)
output="$repo_root/tests/fixtures/ch32v003f4p6-blink.elf"
hello_output="$repo_root/tests/fixtures/ch32v003f4p6-uart-hello.elf"

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

"$cc" --version | sed -n '1p'

mkdir -p "$(dirname -- "$output")"
build_dir=$(mktemp -d)
trap 'rm -rf "$build_dir"' EXIT HUP INT TERM

# Compile to named object files before linking. Linking sources in one GCC
# invocation gives the temporary cc*.o file a random name in the ELF symbol
# table; named intermediate objects keep the checked-in fixture reproducible.
"$cc" \
    -march=rv32e -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -c -o "$build_dir/blink-start.o" "$example_dir/start.S"
"$cc" \
    -march=rv32e -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -c -o "$build_dir/blink-main.o" "$example_dir/main.c"
"$cc" \
    -march=rv32e -mabi=ilp32e -nostdlib -nostartfiles \
    -Wl,-T,"$example_dir/link.x" -Wl,--no-relax -Wl,--build-id=none \
    -o "$output" "$build_dir/blink-start.o" "$build_dir/blink-main.o"

printf 'Built %s\n' "$output"

# The UART hello targets RV32EC, the chip's full ISA, so it also exercises
# compressed instructions on the RV32E register file.
"$cc" \
    -march=rv32ec -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -c -o "$build_dir/uart-start.o" "$example_dir/start.S"
"$cc" \
    -march=rv32ec -mabi=ilp32e -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -c -o "$build_dir/uart-hello.o" "$example_dir/uart_hello.c"
"$cc" \
    -march=rv32ec -mabi=ilp32e -nostdlib -nostartfiles \
    -Wl,-T,"$example_dir/link.x" -Wl,--no-relax -Wl,--build-id=none \
    -o "$hello_output" "$build_dir/uart-start.o" "$build_dir/uart-hello.o"

printf 'Built %s\n' "$hello_output"
