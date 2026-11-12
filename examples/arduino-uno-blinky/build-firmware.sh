#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

pio run -d examples/arduino-uno-blinky
cp examples/arduino-uno-blinky/.pio/build/uno/firmware.elf \
  tests/fixtures/avr/arduino-uno-blinky.elf
cp examples/arduino-uno-blinky/.pio/build/uno/firmware.hex \
  tests/fixtures/avr/arduino-uno-blinky.hex
