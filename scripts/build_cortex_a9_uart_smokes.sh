#!/usr/bin/env bash
# Build the direct A32 Cortex-A9 UART fixtures for the imported Zynq-7000,
# Versatile Express and Tegra 2/3 Renode profiles.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
examples="${repo_root}/examples/renode-onboarding"

for name in zynq7000 vexpress tegra; do
  output_file="${repo_root}/tests/fixtures/${name}-a9-uart-smoke.elf"
  arm-none-eabi-gcc \
    -mcpu=cortex-a9 \
    -marm \
    -nostdlib \
    -nostartfiles \
    -Wl,--build-id=none \
    -Wl,-T,"${examples}/${name}-a9-uart-smoke.ld" \
    -o "${output_file}" \
    "${examples}/${name}-a9-uart-smoke.S"
  chmod 0644 "${output_file}"
done
