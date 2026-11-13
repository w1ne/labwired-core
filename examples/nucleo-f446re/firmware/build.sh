#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
dir="$root/examples/nucleo-f446re/firmware"
out="${1:-$root/tests/fixtures/board-smoke/stm32f446re.elf}"
mkdir -p "$(dirname "$out")"
arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -Os \
  -ffreestanding -fno-builtin -fno-common -fdata-sections -ffunction-sections \
  -nostartfiles -nostdlib -Wl,--gc-sections -Wl,--build-id=none \
  -Wl,-T,"$dir/linker.ld" "$dir/main.c" -o "$out"
echo "firmware=$out"
sha256sum "$out"
