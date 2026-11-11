#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
pio_root="${PLATFORMIO_CORE_DIR:-${HOME}/.platformio}"
toolchain_bin="${pio_root}/packages/toolchain-xtensa-esp-elf/bin"
compiler="${toolchain_bin}/xtensa-esp32s2-elf-gcc"

if [[ ! -x "${compiler}" ]]; then
  command -v pio >/dev/null || {
    echo "PlatformIO is required to provision the ESP32-S2 cross compiler" >&2
    exit 1
  }
  pio platform install espressif32@6.12.0
fi

if [[ ! -x "${compiler}" ]]; then
  echo "ESP32-S2 compiler missing after PlatformIO platform install: ${compiler}" >&2
  exit 1
fi

fixture="${repo_root}/tests/fixtures/esp32s2-devkitc-1-n8r2-uart.elf"
temp_dir="$(mktemp -d)"
trap 'rm -rf "${temp_dir}"' EXIT

"${compiler}" \
  -mlongcalls \
  -mtext-section-literals \
  -Os \
  -ffreestanding \
  -fno-builtin \
  -fdata-sections \
  -ffunction-sections \
  -Wall -Wextra -Werror \
  -c "${script_dir}/main.c" \
  -o "${temp_dir}/main.o"

"${compiler}" \
  -nostdlib \
  -Wl,--build-id=none \
  -Wl,--gc-sections \
  -Wl,-Map,"${temp_dir}/firmware.map" \
  -Wl,-T,"${script_dir}/memory.x" \
  -o "${fixture}" \
  "${temp_dir}/main.o" \
  -lgcc

"${toolchain_bin}/xtensa-esp32s2-elf-size" "${fixture}"
