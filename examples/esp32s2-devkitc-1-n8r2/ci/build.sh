#!/usr/bin/env bash
# Build the simulator used by the committed ESP32-S2 UART fixture.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
