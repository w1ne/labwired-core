#!/bin/bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Pre-commit hook to run fast local validations aligned with CI integrity gates.

set -euo pipefail

echo "--- [Running Pre-commit Validations] ---"

echo "Checking formatting..."
cargo fmt --all -- --check

echo "Running Clippy (all targets)..."
cargo clippy --all-targets -- -D warnings

echo "Running Cargo Check (Host)..."
cargo check --workspace --exclude firmware-stm32f103-uart --exclude firmware-armv6m-ci-fixture --exclude firmware-rv32i-ci-fixture

# Optionally run firmware checks if needed, but these might be slow
# echo "Running Clippy (Firmware)..."
# cargo clippy -p firmware-stm32f103-uart --target thumbv7m-none-eabi -- -D warnings

echo "--- [All Validations Passed!] ---"
exit 0
