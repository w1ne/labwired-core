#!/usr/bin/env bash
# SAM E70 Xplained (ATSAME70Q21B, Cortex-M7) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-sam-e70-xplained-smoke --release --target thumbv7em-none-eabi
