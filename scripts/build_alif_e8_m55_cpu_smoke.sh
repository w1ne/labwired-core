#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_PREFIX="${ARM_NONE_EABI_PREFIX:-arm-none-eabi-}"
SOURCE="${REPO_ROOT}/examples/renode-onboarding/alif-e8-m55-cpu-smoke.S"
LINKER_SCRIPT="${REPO_ROOT}/examples/renode-onboarding/alif-e8-m55-cpu-smoke.ld"
OUTPUT="${REPO_ROOT}/tests/fixtures/alif-e8-m55-cpu-smoke.elf"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "${BUILD_DIR}"' EXIT

"${TOOLCHAIN_PREFIX}gcc" \
  -mcpu=cortex-m33 -mthumb -nostdlib \
  -Wl,--build-id=none -Wl,-T,"${LINKER_SCRIPT}" \
  "${SOURCE}" -o "${OUTPUT}"
chmod 0644 "${OUTPUT}"
"${TOOLCHAIN_PREFIX}readelf" -h "${OUTPUT}" | rg 'Class:|Machine:|Entry point address:'
