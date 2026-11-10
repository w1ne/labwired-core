#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pio run \
  --project-dir "${repo_root}/examples/esp32-devkitc-32e/firmware" \
  --environment devkitc32e_mcpwm
