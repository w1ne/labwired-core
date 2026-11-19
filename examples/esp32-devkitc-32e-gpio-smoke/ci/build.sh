#!/usr/bin/env bash
# The TIER1 ELF is committed under tests/fixtures/tier1/, so there is no
# firmware build step here. Build the simulator instead, which is what
# ci/test.sh runs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
