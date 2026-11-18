#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-zero-m-bme280/bme280-smoke.yaml \
  --output-dir out/boards/esp32c6-zero-m-bme280/bme280-smoke \
  --no-uart-stdout
