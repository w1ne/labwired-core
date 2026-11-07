#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
for part in atmega8 atmega48 atmega88 atmega32 atmega164p atmega324p atmega644p; do
  ${AVR_CC:-avr-gcc} -mmcu="$part" -Os -Wall -Wextra -Werror \
    -o "tests/fixtures/avr/${part%p}-cpu-smoke.elf" \
    tests/fixtures/avr/attiny-x5-cpu-smoke.c
done
for part in atmega169p atmega16m1 atmega64m1 atmega128rfa1 atmega128rfr2 at90usb162; do
  ${AVR_CC:-avr-gcc} -mmcu="$part" -Os -Wall -Wextra -Werror \
    -o "tests/fixtures/avr/${part}-cpu-smoke.elf" \
    tests/fixtures/avr/attiny-x5-cpu-smoke.c
done
