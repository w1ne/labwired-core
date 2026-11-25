#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
scripts/onboarding_smoke.sh \
  --target-id esp32p4x-c5-function-ev-board-bme280 \
  --script examples/esp32p4x-c5-function-ev-board/io-smoke.yaml \
  --system configs/systems/esp32p4x-c5-function-ev-board.yaml \
  --firmware-build-script examples/esp32p4x-c5-function-ev-board/ci/build.sh \
  --out-dir examples/esp32p4x-c5-function-ev-board/evidence/run
