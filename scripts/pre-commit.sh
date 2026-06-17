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
cargo check --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture

# Keep the generated board validation status in sync, and block the commit if a
# peripheral model drifted past its last silicon capture without a drift_ack.
if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" >/dev/null 2>&1; then
  echo "Regenerating board validation status + drift gate..."
  python3 scripts/generate_validation_status.py
  python3 scripts/generate_validation_status.py --drift
  git add docs/boards/VALIDATION_STATUS.md
else
  echo "(skip validation-status: python3 + pyyaml not available)"
fi

# Optionally run firmware checks if needed, but these might be slow
# echo "Running Clippy (Firmware)..."
# cargo clippy -p firmware --target thumbv7m-none-eabi -- -D warnings

echo "--- [All Validations Passed!] ---"
exit 0
