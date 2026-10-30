#!/usr/bin/env bash
# NUCLEO-L552ZE-Q (STM32L552ZET6Q, Cortex-M33) smoke firmware build.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi
