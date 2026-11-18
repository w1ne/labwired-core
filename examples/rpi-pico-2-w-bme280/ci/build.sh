#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT/crates/firmware-pico2w-bme280-smoke"
cargo build --release
