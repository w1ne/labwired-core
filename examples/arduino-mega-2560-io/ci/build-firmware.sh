#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(git -C "${HERE}" rev-parse --show-toplevel)"
BUILD_ELF="${HERE}/.pio/build/megaatmega2560/firmware.elf"
FIXTURE="${ROOT}/tests/fixtures/avr/arduino-mega-2560-io.elf"

pio run --project-dir "${HERE}" --environment megaatmega2560
test -s "${BUILD_ELF}"
cp "${BUILD_ELF}" "${FIXTURE}"
sha256sum "${FIXTURE}"
