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
  event_clr_deasserts_on_walk_deleted -- --quiet
cargo test -p labwired-core --features event-scheduler --lib \
  tests::rp2040 -- --quiet

echo "LogicTap unit smokes..."
cargo test -p labwired-core --lib logic_tap_sees -- --quiet

# The class of failure that burned a CI round: a data refresh that clobbers
# curated descriptors (IRQs, register-layout profiles, memory bases) loads fine
# and validates fine, but breaks the onboarding contracts. These four suites are
# the ones that caught it; they run in seconds once the test binaries exist.
echo "Onboarding descriptor integrity (unknown types + curated-data contracts)..."
cargo test -p labwired-core --test unknown_peripheral_type -- --quiet
cargo test -p labwired-core --lib tests::nrf52 -- --quiet
cargo test -p labwired-core --lib tests::peripheral_reachability -- --quiet
cargo test -p labwired-core --lib tests::yaml_owned_base_contract -- --quiet

if [[ "${LABWIRED_PREPUSH_FULL:-}" == "1" ]]; then
  echo "--- [LABWIRED_PREPUSH_FULL=1: workspace tests] ---"
  cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
fi

echo "--- [Pre-push passed] ---"
echo "Tip: set LABWIRED_PREPUSH_FULL=1 for full workspace tests."
exit 0
