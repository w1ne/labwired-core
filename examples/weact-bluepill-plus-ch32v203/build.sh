#!/bin/sh
# Build the WeAct BluePill Plus (CH32V203) template fixture. It reuses the
# CH32V203C8T6 start-up and linker script; any RISC-V GCC with RV32IMAC works.
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$example_dir/../.." && pwd)
chip_dir="$repo_root/examples/ch32v203c8t6"
output="$repo_root/tests/fixtures/weact-bluepill-plus-ch32v203-template.elf"

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
"$cc" \
    -march=rv32imac -mabi=ilp32 -Os \
    -ffreestanding -fno-builtin -fno-pic -fno-pie \
    -nostdlib -nostartfiles \
    -Wl,-T,"$chip_dir/link.x" -Wl,--no-relax -Wl,--build-id=none \
    -o "$output" \
    "$chip_dir/start.S" "$example_dir/weact_template.c"
printf 'Built %s\n' "$output"
