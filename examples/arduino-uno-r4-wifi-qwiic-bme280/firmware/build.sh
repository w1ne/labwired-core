#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FIRMWARE_DIR="${ROOT}/examples/arduino-uno-r4-wifi-qwiic-bme280/firmware"
ARTIFACT_DIR="${ROOT}/target/arduino-uno-r4-wifi-qwiic-bme280"

pio run --project-dir "${FIRMWARE_DIR}" -e qwiic
mkdir -p "${ARTIFACT_DIR}"
install -m 0644 \
  "${FIRMWARE_DIR}/.pio/build/qwiic/firmware.elf" \
  "${ARTIFACT_DIR}/firmware.elf"
