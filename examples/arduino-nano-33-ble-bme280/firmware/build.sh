#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
pio run -d "${script_dir}" -e nano33ble
install -m 644 "${script_dir}/.pio/build/nano33ble/firmware.elf" \
  "${repo_root}/tests/fixtures/nrf52/arduino-nano-33-ble-bme280.elf"
