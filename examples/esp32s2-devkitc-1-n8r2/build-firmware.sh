#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
firmware_dir="$repo_root/examples/esp32s2-devkitc-1-n8r2/firmware"
fixture="$repo_root/tests/fixtures/esp32s2-devkitc-1-n8r2-uart.elf"
compiler="${CC:-$HOME/.platformio/packages/toolchain-xtensa-esp-elf/bin/xtensa-esp32s2-elf-gcc}"

if [[ ! -x "$compiler" ]]; then
    echo "Xtensa ESP32-S2 GCC not found at $compiler; install the Espressif Xtensa toolchain or set CC." >&2
    exit 1
fi

# Use the S2-targeted toolchain so ELF data and instruction byte order match
# the little-endian ESP32-S2. The generic Xtensa compiler emits big-endian ELF.
"$compiler" -mabi=call0 -mlongcalls -ffreestanding -fno-builtin \
    -fno-stack-protector -Os -Wall -Wextra -nostdlib -nostartfiles \
    -Wl,--gc-sections -Wl,-T,"$firmware_dir/esp32s2.ld" \
    "$firmware_dir/main.c" -o "$fixture"

echo "Built ESP32-S2 UART fixture: $fixture"
sha256sum "$fixture"
