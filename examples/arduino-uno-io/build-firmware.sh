#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

pio run -d examples/arduino-uno-io
cp examples/arduino-uno-io/.pio/build/uno/firmware.elf \
  tests/fixtures/avr/arduino-uno-io.elf
