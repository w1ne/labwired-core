#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
${AVR_CC:-avr-gcc} -mmcu=atmega168 -Os -Wall -Wextra -Werror \
  -o tests/fixtures/avr/atmega168-cpu-smoke.elf \
  tests/fixtures/avr/atmega168-cpu-smoke.c
