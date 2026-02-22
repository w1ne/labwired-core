#!/bin/bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Pre-push hook for deeper validation before remote update.

set -euo pipefail

echo "--- [Running Pre-push Validations] ---"

echo "Checking formatting..."
cargo fmt --all -- --check

echo "Running Clippy (all targets)..."
cargo clippy --all-targets -- -D warnings

echo "Running workspace tests..."
cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture

echo "--- [All Pre-push Validations Passed!] ---"
exit 0

