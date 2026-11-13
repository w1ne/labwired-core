#!/usr/bin/env bash
# Build the direct A32 AT91RM9200 Debug Unit fixture for the imported Renode
# profile. It is assembled for the silicon's ARM920T (ARMv4T) so it does not
# depend on the ARMv5TEJ extras of the ARM926EJ-S model it runs on.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
examples="${repo_root}/examples/renode-onboarding"
output_file="${repo_root}/tests/fixtures/at91rm9200-dbgu-smoke.elf"

arm-none-eabi-gcc \
  -mcpu=arm920t \
  -marm \
  -nostdlib \
  -nostartfiles \
  -Wl,--build-id=none \
  -Wl,-T,"${examples}/at91rm9200-dbgu-smoke.ld" \
  -o "${output_file}" \
  "${examples}/at91rm9200-dbgu-smoke.S"
chmod 0644 "${output_file}"
