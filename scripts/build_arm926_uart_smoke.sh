#!/usr/bin/env bash
# Build the direct A32 ARM926EJ-S UART fixture for the imported Versatile/PB
# Renode profile.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
examples="${repo_root}/examples/renode-onboarding"
output_file="${repo_root}/tests/fixtures/versatile-arm926-uart-smoke.elf"

arm-none-eabi-gcc \
  -mcpu=arm926ej-s \
  -marm \
  -nostdlib \
  -nostartfiles \
  -Wl,--build-id=none \
  -Wl,-T,"${examples}/versatile-arm926-uart-smoke.ld" \
  -o "${output_file}" \
  "${examples}/versatile-arm926-uart-smoke.S"
chmod 0644 "${output_file}"
