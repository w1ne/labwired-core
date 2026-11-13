#!/usr/bin/env bash
# EK-RA6M5 (R7FA6M5BH, Cortex-M33) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-ek-ra6m5-smoke --release --target thumbv8m.main-none-eabi
