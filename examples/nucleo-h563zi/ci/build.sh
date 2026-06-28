#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
cargo build -p firmware-h563-fullchip-demo --release --target thumbv7m-none-eabi
