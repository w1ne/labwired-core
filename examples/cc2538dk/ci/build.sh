#!/usr/bin/env bash
# CC2538DK (CC2538SF53, Cortex-M3) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-cc2538dk-smoke --release --target thumbv7m-none-eabi
