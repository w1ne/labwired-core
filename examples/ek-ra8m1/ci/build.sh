#!/usr/bin/env bash
# EK-RA8M1 (R7FA8M1AH, Cortex-M85) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-ek-ra8m1-smoke --release --target thumbv8m.main-none-eabi
