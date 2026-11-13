#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
${AVR_CC:-avr-gcc} -mmcu=atmega128 -Os -Wall -Wextra -Werror \
  -o tests/fixtures/avr/atmega128-cpu-smoke.elf \
  tests/fixtures/avr/atmega128-cpu-smoke.c
