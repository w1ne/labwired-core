#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
examples/esp32s3-lcd-ev-board/ci/verify-fixture.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s3-lcd-ev-board/touch-smoke.yaml \
  --output-dir out/boards/esp32s3-lcd-ev-board \
  --no-uart-stdout
