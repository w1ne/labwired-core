#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
(cd crates/firmware-adafruit-feather-rp2350-smoke && \
  cargo build -p firmware-adafruit-feather-rp2350-smoke \
    --release --target thumbv8m.main-none-eabi)
