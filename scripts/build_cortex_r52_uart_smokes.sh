#!/usr/bin/env bash
# Build the direct A32 Cortex-R52 fixtures for the imported Renode
# cortex-r52 (PL011 UART0) and RZ/T2M (SCI0) profiles.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
examples="${repo_root}/examples/renode-onboarding"

for name in cortex-r52-uart rz-t2m-r52-sci; do
  output_file="${repo_root}/tests/fixtures/${name}-smoke.elf"
  arm-none-eabi-gcc \
    -mcpu=cortex-r52 \
    -marm \
    -nostdlib \
    -nostartfiles \
    -Wl,--build-id=none \
    -Wl,-T,"${examples}/${name}-smoke.ld" \
    -o "${output_file}" \
    "${examples}/${name}-smoke.S"
  chmod 0644 "${output_file}"
done
