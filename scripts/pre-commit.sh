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
# Reuse CI's crate-declared host/firmware classification. The clippy complement
# alone omits default-members and wasm; the host check must include all of them.
# Command substitution propagates classifier errors (process substitution does
# not), and the read loop also works with macOS's Bash 3.2.
host_scope="$(python3 scripts/ci/host_clippy_scope.py --host-cargo-args)"
if [[ -z "$host_scope" ]]; then
  echo "Host scope is empty; refusing to check only default-members." >&2
  exit 1
fi
host_args=()
while IFS= read -r package; do
  host_args+=("$package")
done <<< "$host_scope"
cargo check "${host_args[@]}"

# Keep board validation status current and enforce source-watch integrity.
# Source/capture differences are reported; no human source-review ACK is required.
if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" >/dev/null 2>&1; then
  echo "Regenerating board validation status + source/capture report..."
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
