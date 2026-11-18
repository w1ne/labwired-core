#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
cargo build -q -p firmware-stm32f103-lowdensity-smoke --release --target thumbv7m-none-eabi
