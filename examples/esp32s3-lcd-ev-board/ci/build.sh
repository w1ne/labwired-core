#!/usr/bin/env bash
# Build the simulator used by ci/test.sh; the cross-built firmware fixture is
# committed and verified separately.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
cargo build -q -p labwired-cli
