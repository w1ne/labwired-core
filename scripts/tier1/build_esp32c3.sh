#!/usr/bin/env bash
# Build the ESP32-C3 Tier-1 fixture ELF and copy it to the fixtures directory.
#
# Usage: scripts/tier1/build_esp32c3.sh [--copy]
#
# With --copy (or when TIER1_COPY=1), the built ELF is copied to
# tests/fixtures/tier1/esp32c3.elf so it can be committed.
#
# Prerequisites:
#   rustup target add riscv32imc-unknown-none-elf
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURE_DIR="${REPO_ROOT}/examples/tier1-fixture/esp32c3"
FIXTURE_ELF="${FIXTURE_DIR}/target/riscv32imc-unknown-none-elf/release/tier1-fixture-esp32c3"
DEST_ELF="${REPO_ROOT}/tests/fixtures/tier1/esp32c3.elf"

# Ensure the riscv32imc target is available.
if ! rustup target list --installed 2>/dev/null | grep -q riscv32imc-unknown-none-elf; then
    echo "Installing riscv32imc-unknown-none-elf target..."
    rustup target add riscv32imc-unknown-none-elf
fi

echo "Building tier1-fixture-esp32c3..."
(
    cd "${FIXTURE_DIR}"
    cargo build --release
)

echo "Built: ${FIXTURE_ELF}"

if [[ "${1:-}" == "--copy" ]] || [[ "${TIER1_COPY:-0}" == "1" ]]; then
    mkdir -p "$(dirname "${DEST_ELF}")"
    cp "${FIXTURE_ELF}" "${DEST_ELF}"
    echo "Copied to: ${DEST_ELF}"
fi
