#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pio run \
  --project-dir "${repo_root}/examples/adafruit-feather-esp32-v2/firmware" \
  --environment feather_bme280
