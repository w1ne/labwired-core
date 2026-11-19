#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"
cargo build -p firmware-esp32c5-devkitc-1-bme280-smoke \
  --release --target riscv32imac-unknown-none-elf

# Some worktrees use a shared CARGO_TARGET_DIR outside the checkout. Stage the
# ELF in this example's ignored build directory so both local and CI runners
# can resolve the same stable path from the smoke YAML.
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  case "${CARGO_TARGET_DIR}" in
    /*) CARGO_OUTPUT_ROOT="${CARGO_TARGET_DIR}" ;;
    *) CARGO_OUTPUT_ROOT="${REPO_ROOT}/${CARGO_TARGET_DIR}" ;;
  esac
else
  CARGO_OUTPUT_ROOT="${REPO_ROOT}/target"
fi

FIRMWARE_NAME="firmware-esp32c5-devkitc-1-bme280-smoke"
BUILT_FIRMWARE="${CARGO_OUTPUT_ROOT}/riscv32imac-unknown-none-elf/release/${FIRMWARE_NAME}"
EXPECTED_FIRMWARE="${REPO_ROOT}/examples/esp32c5-devkitc-1-bme280-smoke/build/firmware.elf"
mkdir -p "$(dirname "${EXPECTED_FIRMWARE}")"
cp "${BUILT_FIRMWARE}" "${EXPECTED_FIRMWARE}"
