#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
pio run --project-dir "${REPO_ROOT}/examples/adafruit-esp32-c6-feather-bme280/firmware" \
  --environment feather_c6
