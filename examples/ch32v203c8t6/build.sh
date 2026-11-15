#!/bin/sh
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$example_dir/../.." && pwd)
output="$repo_root/tests/fixtures/ch32v203c8t6-blink.elf"
hello_output="$repo_root/tests/fixtures/ch32v203c8t6-uart-hello.elf"

if [ -n "${RISCV_CC:-}" ]; then
    cc=$RISCV_CC
elif command -v riscv-none-elf-gcc >/dev/null 2>&1; then
    cc=riscv-none-elf-gcc
elif command -v riscv64-unknown-elf-gcc >/dev/null 2>&1; then
    cc=riscv64-unknown-elf-gcc
elif command -v riscv32-esp-elf-gcc >/dev/null 2>&1; then
    cc=riscv32-esp-elf-gcc
else
    echo "RISC-V GCC with RV32IMAC support not found; set RISCV_CC" >&2
    exit 127
fi

mkdir -p "$(dirname -- "$output")"
for pair in "main.c:$output" "uart_hello.c:$hello_output"; do
    src=${pair%%:*}
    out=${pair#*:}
    "$cc" \
        -march=rv32imac -mabi=ilp32 -Os \
        -ffreestanding -fno-builtin -fno-pic -fno-pie \
        -nostdlib -nostartfiles \
        -Wl,-T,"$example_dir/link.x" -Wl,--no-relax \
        -o "$out" \
        "$example_dir/start.S" "$example_dir/$src"
    printf 'Built %s\n' "$out"
done
