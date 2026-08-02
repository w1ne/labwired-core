#!/usr/bin/env bash
# The ESP32-C3 blinky ELF is committed under firmware/, so there is no firmware
# build step here. Build the simulator instead, which is what ci/test.sh runs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
