#!/usr/bin/env bash
# The smoke ELF is committed under tests/fixtures/ (rebuild it with
# examples/rp2354b/firmware/build.sh), so there is no firmware build step
# here. Build the simulator instead, which is what ci/test.sh runs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
