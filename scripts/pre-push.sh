#!/bin/bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Pre-push hook: catch the failures that burned multi-hour CI loops
# (clippy, RP2040 bare-metal+bootrom, C3 walk-deletion) before remote update.
#
# Fast by default. Opt into the old heavy suite with:
#   LABWIRED_PREPUSH_FULL=1 git push

set -euo pipefail

echo "--- [Pre-push: fast gate] ---"

echo "Checking formatting..."
cargo fmt --all -- --check

echo "Clippy (lib targets; -D warnings)..."
cargo clippy -p labwired-core -p labwired-cli --all-targets -- -D warnings

echo "Walk-deletion + RP2040 vector tests (event-scheduler feature)..."
cargo test -p labwired-core --features event-scheduler --lib \
  event_clr_deasserts_on_walk_deleted \
  tests::rp2040 \
  -- --quiet

echo "LogicTap unit smokes..."
cargo test -p labwired-core --lib logic_tap_sees -- --quiet

if [[ "${LABWIRED_PREPUSH_FULL:-}" == "1" ]]; then
  echo "--- [LABWIRED_PREPUSH_FULL=1: workspace tests] ---"
  cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
fi

echo "--- [Pre-push passed] ---"
echo "Tip: set LABWIRED_PREPUSH_FULL=1 for full workspace tests."
exit 0
