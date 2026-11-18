#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"
cargo build -p firmware-esp32c61-demo --release --target riscv32imc-unknown-none-elf

# Stage the ELF at a stable path when this worktree uses an external cargo
# target cache. The smoke YAML resolves build/firmware.elf from this example.
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  case "${CARGO_TARGET_DIR}" in
    /*) CARGO_OUTPUT_ROOT="${CARGO_TARGET_DIR}" ;;
    *) CARGO_OUTPUT_ROOT="${REPO_ROOT}/${CARGO_TARGET_DIR}" ;;
  esac
else
  CARGO_OUTPUT_ROOT="${REPO_ROOT}/target"
fi

BUILT_FIRMWARE="${CARGO_OUTPUT_ROOT}/riscv32imc-unknown-none-elf/release/firmware-esp32c61-demo"
STAGED_FIRMWARE="${REPO_ROOT}/examples/esp32c61-devkitc-1-n8r2/build/firmware.elf"
mkdir -p "$(dirname "${STAGED_FIRMWARE}")"
cp "${BUILT_FIRMWARE}" "${STAGED_FIRMWARE}"
