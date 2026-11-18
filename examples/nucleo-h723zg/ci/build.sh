#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
cargo build -p firmware-nucleo-h723zg-smoke --release --target thumbv7em-none-eabi
