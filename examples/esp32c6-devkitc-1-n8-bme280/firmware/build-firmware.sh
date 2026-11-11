#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
pio run --project-dir "${REPO_ROOT}/examples/esp32c6-devkitc-1-n8-bme280/firmware" \
  --environment devkitc_n8
