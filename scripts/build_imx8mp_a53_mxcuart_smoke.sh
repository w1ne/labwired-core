#!/usr/bin/env bash
# Builds the i.MX 8M Plus A53 UART smoke twice: UART2 (nxp-mimx8ml) and
# UART4 (nxp-frdm-imx8mplus), matching each Renode script's console UART.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_PREFIX="${AARCH64_ZEPHYR_ELF_PREFIX:-$HOME/tools/aarch64-zephyr-elf-dl/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-}"
SOURCE="${REPO_ROOT}/examples/renode-onboarding/imx8mp-a53-mxcuart-smoke.S"
LINKER_SCRIPT="${REPO_ROOT}/examples/renode-onboarding/imx8mp-a53-mxcuart-smoke.ld"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "${BUILD_DIR}"' EXIT

build() {
  local name="$1" base="$2"
  local output="${REPO_ROOT}/tests/fixtures/imx8mp-a53-${name}-smoke.elf"
  "${TOOLCHAIN_PREFIX}gcc" -c "-Wa,--defsym,UART_BASE=${base}" \
    -o "${BUILD_DIR}/${name}.o" "${SOURCE}"
  "${TOOLCHAIN_PREFIX}ld" \
    --build-id=none \
    -T "${LINKER_SCRIPT}" \
    -o "${output}" \
    "${BUILD_DIR}/${name}.o"
  chmod 0644 "${output}"
  "${TOOLCHAIN_PREFIX}readelf" -h "${output}" | grep -E 'Class:|Machine:|Entry point address:'
}

build uart2 0x30890000
build uart4 0x30A60000
