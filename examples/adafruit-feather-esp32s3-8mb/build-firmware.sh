#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pio run --project-dir "$ROOT/examples/adafruit-feather-esp32s3-8mb/firmware" \
  --environment feather_nopsram
