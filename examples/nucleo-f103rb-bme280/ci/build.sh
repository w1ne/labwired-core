#!/usr/bin/env bash
# Build the simulator and the committed-source BME280 smoke firmware (no
# extra C toolchain — rustup adds thumbv7m-none-eabi via boards.yml).
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
cargo build -q -p firmware-nucleo-f103rb-bme280-smoke --release --target thumbv7m-none-eabi
