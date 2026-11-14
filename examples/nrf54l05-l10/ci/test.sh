#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p out/boards/nrf54l05-l10-smoke
cargo run -q -p labwired-cli -- test \
  --script examples/nrf54l05-l10/nrf54l05-smoke.yaml \
  --output-dir out/boards/nrf54l05-l10-smoke/l05 \
  --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/nrf54l05-l10/nrf54l10-smoke.yaml \
  --output-dir out/boards/nrf54l05-l10-smoke/l10 \
  --no-uart-stdout
