#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
pio run -d "${script_dir}" -e nano_33_iot
install -m 644 "${script_dir}/.pio/build/nano_33_iot/firmware.elf" \
  "${repo_root}/tests/fixtures/samd/arduino-nano-33-iot-bme280.elf"
