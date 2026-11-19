#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
pio run -d "${script_dir}" -e nano_every
cp "${script_dir}/.pio/build/nano_every/firmware.elf" \
  "${repo_root}/tests/fixtures/avr/arduino-nano-every-max31855.elf"
