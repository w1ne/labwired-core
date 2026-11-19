#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

pio run -d examples/arduino-nano-blinky
cp examples/arduino-nano-blinky/.pio/build/nano/firmware.elf \
  tests/fixtures/avr/arduino-nano-blinky.elf
