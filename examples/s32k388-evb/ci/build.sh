#!/usr/bin/env bash
# S32K388EVB-Q289 (S32K388, CM7_0 Cortex-M7) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-s32k388-evb-smoke --release --target thumbv7em-none-eabi
