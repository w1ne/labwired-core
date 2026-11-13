#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
${AVR_CC:-avr-gcc} -mmcu=atmega16 -Os -Wall -Wextra -Werror \
  -o tests/fixtures/avr/atmega16-cpu-smoke.elf \
  tests/fixtures/avr/atmega16-cpu-smoke.c
