#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cargo build \
  --manifest-path "${ROOT}/crates/firmware-nucleo-u545re-smoke/Cargo.toml" \
  --bin firmware-nucleo-u545re-bme280 \
  --target thumbv8m.main-none-eabi \
  --release
