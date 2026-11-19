#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_file="${repo_root}/examples/renode-onboarding/xtensa-sample-controller-cpu-smoke.S"
linker_script="${repo_root}/examples/renode-onboarding/xtensa-sample-controller-cpu-smoke.ld"
output_file="${repo_root}/tests/fixtures/xtensa-sample-controller-cpu-smoke.elf"
# Any Xtensa GNU toolchain assembles this core-ISA source; the ESP32 one
# (PlatformIO toolchain-xtensa-esp-elf) is the one this repository uses.
cc="${XTENSA_CC:-xtensa-esp32-elf-gcc}"

"${cc}" \
  -nostdlib \
  -nostartfiles \
  -mtext-section-literals \
  -Wl,--build-id=none \
  -Wl,-T,"${linker_script}" \
  -o "${output_file}" \
  "${source_file}"
chmod 0644 "${output_file}"
