#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pio run --project-dir "$root/examples/esp32s3-zero/firmware" --environment zero
