#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_PREFIX="${AARCH64_ZEPHYR_ELF_PREFIX:-$HOME/tools/aarch64-zephyr-elf-dl/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-}"
SOURCE="${REPO_ROOT}/examples/renode-onboarding/virt-a53-pl011-smoke.S"
LINKER_SCRIPT="${REPO_ROOT}/examples/renode-onboarding/virt-a53-pl011-smoke.ld"
OUTPUT="${REPO_ROOT}/tests/fixtures/virt-a53-pl011-smoke.elf"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "${BUILD_DIR}"' EXIT

"${TOOLCHAIN_PREFIX}gcc" -c -o "${BUILD_DIR}/smoke.o" "${SOURCE}"
"${TOOLCHAIN_PREFIX}ld" \
  --build-id=none \
  -T "${LINKER_SCRIPT}" \
  -o "${OUTPUT}" \
  "${BUILD_DIR}/smoke.o"
chmod 0644 "${OUTPUT}"
"${TOOLCHAIN_PREFIX}readelf" -h "${OUTPUT}" | grep -E 'Class:|Machine:|Entry point address:'
