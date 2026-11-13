#!/usr/bin/env bash
# What pr-gate will check, runnable before you push.
#
# WHY THIS EXISTS
#
# The gates below are each cheap and each catch a whole class of CI round-trip.
# Reconstructing the list from memory before every push does not work: over one
# branch it missed rustfmt once, the generated-artifact regen twice, and clippy
# once -- four red CI runs, none of them a defect in the code, all of them a
# gap in the routine.
#
# It is also easy to write a check whose exit code is not the check's. This
# script uses no pipes into grep for that reason: every command's own status is
# what counts, and `set -e` stops at the first failure.
#
# NOT a substitute for CI. The workspace shards, the differential gates and the
# perf lane still run there. This is the subset that is fast enough to run
# every time and that has actually gone red.

set -euo pipefail
cd "$(dirname "$0")/../.."

step() { printf '\n=== %s ===\n' "$1"; }

step "rustfmt"
cargo fmt --all -- --check

step "clippy (the pr-gate command)"
cargo clippy --all-targets -- -D warnings

step "unit tests, FEATURE-OFF (the crate's default, which no CI lane ran until now)"
cargo test -p labwired-core --lib

step "unit tests, event-scheduler"
cargo test -p labwired-core --features event-scheduler --lib

# --drift reports source/capture differences; integrity and --check still fail on errors.
step "generated artefacts are not stale"
python3 scripts/generate_validation_status.py --check --drift
python3 scripts/generate_firmware_exercise_matrix.py --check

printf '\npre-push: all green\n'
