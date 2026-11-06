#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_file="${repo_root}/examples/renode-onboarding/a20-a7-uart-smoke.S"
linker_script="${repo_root}/examples/renode-onboarding/a20-a7-uart-smoke.ld"
output_file="${repo_root}/tests/fixtures/a20-a7-uart-smoke.elf"

arm-none-eabi-gcc \
  -mcpu=cortex-a7 \
  -marm \
  -nostdlib \
  -nostartfiles \
  -Wl,--build-id=none \
  -Wl,-T,"${linker_script}" \
  -o "${output_file}" \
  "${source_file}"
chmod 0644 "${output_file}"
