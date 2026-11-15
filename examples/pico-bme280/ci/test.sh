#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "${repo_root}"
examples/pico-bme280/verify-fixture.sh
cargo run -q -p labwired-cli -- test \
  --script examples/pico-bme280/bme280-smoke.yaml \
  --output-dir out/boards/pico-bme280 \
  --no-uart-stdout
