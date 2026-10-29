#!/usr/bin/env bash
# This example uses a committed AVR ELF; build the simulator that ci/test.sh runs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
