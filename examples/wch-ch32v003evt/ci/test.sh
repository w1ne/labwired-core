#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

cargo run --locked -q -p labwired-cli -- test \
  --script examples/wch-ch32v003evt/io-smoke.yaml \
  --output-dir out/boards/wch-ch32v003evt/io-smoke --no-uart-stdout
cargo run --locked -q -p labwired-cli -- test \
  --script examples/wch-ch32v003evt/uart-smoke.yaml \
  --output-dir out/boards/wch-ch32v003evt/uart-smoke --no-uart-stdout
