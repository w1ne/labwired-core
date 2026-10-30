#!/usr/bin/env bash
# FRDM-K64F (MK64FN1M0VLL12, Cortex-M4F) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-frdm-k64f-smoke --release --target thumbv7em-none-eabi
