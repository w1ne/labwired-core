#!/usr/bin/env bash
# MIMXRT595-EVK (MIMXRT595S, Cortex-M33) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-mimxrt595-evk-smoke --release --target thumbv8m.main-none-eabi
