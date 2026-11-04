#!/usr/bin/env bash
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${EXAMPLE_DIR}/../.." && pwd)"
OUTPUT_DIR="${REPO_ROOT}/target/riscv32imc-unknown-none-elf/release"

CC="$(command -v riscv32-esp-elf-gcc || true)"
if [[ -z "${CC}" ]]; then
  CC="$(find "${HOME}/.platformio/packages" -type f -name riscv32-esp-elf-gcc -print -quit 2>/dev/null || true)"
fi
if [[ -z "${CC}" ]]; then
  echo "riscv32-esp-elf-gcc not found; install the PlatformIO Espressif RISC-V toolchain" >&2
  exit 1
fi

echo "Compiler: ${CC}"
"${CC}" --version | sed -n '1p'
mkdir -p "${OUTPUT_DIR}"
"${CC}" \
  -march=rv32imac -mabi=ilp32 \
  -Os -g -ffreestanding -fno-builtin -fno-stack-protector \
  -ffunction-sections -fdata-sections -nostartfiles -nostdlib \
  -Wl,--build-id=none -Wl,--no-relax -Wl,--gc-sections \
  -T "${EXAMPLE_DIR}/firmware/link.ld" \
  "${EXAMPLE_DIR}/firmware/startup.S" \
  "${EXAMPLE_DIR}/firmware/gpio_smoke.c" \
  -o "${OUTPUT_DIR}/firmware-xiao-esp32c6-gpio-smoke"
echo "Built ${OUTPUT_DIR}/firmware-xiao-esp32c6-gpio-smoke"
