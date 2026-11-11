#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
pio run --project-dir "${REPO_ROOT}/examples/xiao-esp32c6-bme280/firmware" \
  --environment xiao_c6
