#!/usr/bin/env bash
set -euo pipefail
: "${PICO_SDK:?Set PICO_SDK to an unmodified pico-sdk checkout}"
expected=079c6f39023649b154152db30f1d781e884879bc
[[ $(git -C "$PICO_SDK" rev-parse HEAD) == "$expected" ]] || { echo 'Pico SDK commit mismatch' >&2; exit 1; }
git -C "$PICO_SDK" diff --quiet --exit-code
git -C "$PICO_SDK" diff --cached --quiet --exit-code
fixture_dir=$(cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(cd -- "$fixture_dir/../../.." && pwd)
output=${RP2354B_SMOKE_OUT:-$repo_dir/tests/fixtures/rp2354b-smoke.elf}
mkdir -p -- "$(dirname -- "$output")"
cd -- "$fixture_dir"
arm-none-eabi-gcc -mcpu=cortex-m33 -mthumb -mfloat-abi=soft -Os -g0 \
  -ffreestanding -fno-builtin -ffunction-sections -fdata-sections -nostdlib \
  -Wall -Wextra -Werror -I "$PICO_SDK/src/rp2350/hardware_regs/include" \
  -T link.ld -Wl,--gc-sections -Wl,--build-id=none smoke.c -o "$output"
arm-none-eabi-size "$output"
sha256sum "$output"
