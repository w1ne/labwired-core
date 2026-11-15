#!/usr/bin/env bash
# The exact Arduino firmware fixture is committed; build the simulator used by
# ci/test.sh. The hosted onboarding workflow rebuilds the firmware with PIO.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
cargo build -q -p labwired-cli
