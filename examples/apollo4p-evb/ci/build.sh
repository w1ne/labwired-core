#!/usr/bin/env bash
# Apollo4 Plus EVB (AMAP42KP, Cortex-M4F) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-apollo4p-evb-smoke --release --target thumbv7em-none-eabi
