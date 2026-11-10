#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
pio run --project-dir "${REPO_ROOT}/examples/arduino-nano-esp32-bme280/firmware" \
  --environment nano_esp32
