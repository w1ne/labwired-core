#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_file="${repo_root}/tests/fixtures/cortex-r52-cpu-smoke.S"
linker_script="${repo_root}/tests/fixtures/cortex-r52-cpu-smoke.ld"
output_file="${repo_root}/tests/fixtures/cortex-r52-cpu-smoke.elf"

arm-none-eabi-gcc \
  -mcpu=cortex-r5 \
  -marm \
  -nostdlib \
  -nostartfiles \
  -Wl,--build-id=none \
  -Wl,-T,"${linker_script}" \
  -o "${output_file}" \
  "${source_file}"
