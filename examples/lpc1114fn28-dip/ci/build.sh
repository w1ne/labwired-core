#!/usr/bin/env bash
# LPC1114FN28/102 (Cortex-M0) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-lpc1114fn28-smoke --release --target thumbv6m-none-eabi
