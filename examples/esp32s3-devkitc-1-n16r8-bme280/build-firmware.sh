#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
pio run --project-dir "${REPO_ROOT}/examples/esp32s3-devkitc-1-n16r8-bme280/firmware" \
  --environment devkitc_n16r8
