#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_PREFIX="${ARM_NONE_EABI_PREFIX:-arm-none-eabi-}"
SOURCE="${REPO_ROOT}/examples/renode-onboarding/colibri-vf61-cortex-m4-smoke.S"
LINKER_SCRIPT="${REPO_ROOT}/examples/renode-onboarding/colibri-vf61-cortex-m4-smoke.ld"
OUTPUT="${REPO_ROOT}/tests/fixtures/colibri-vf61-cortex-m4-smoke.elf"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "${BUILD_DIR}"' EXIT

"${TOOLCHAIN_PREFIX}gcc" -mcpu=cortex-m4 -mthumb -c \
  -o "${BUILD_DIR}/smoke.o" "${SOURCE}"
"${TOOLCHAIN_PREFIX}ld" --build-id=none -T "${LINKER_SCRIPT}" \
  -o "${OUTPUT}" "${BUILD_DIR}/smoke.o"
chmod 0644 "${OUTPUT}"
"${TOOLCHAIN_PREFIX}readelf" -h "${OUTPUT}" | rg 'Class:|Machine:|Entry point address:'
