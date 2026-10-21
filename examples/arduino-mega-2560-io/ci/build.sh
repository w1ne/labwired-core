#!/usr/bin/env bash
# The Mega 2560 I/O ELF is committed under tests/fixtures/avr/, so there is no
# firmware build step here. Build the simulator instead, which ci/test.sh runs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
