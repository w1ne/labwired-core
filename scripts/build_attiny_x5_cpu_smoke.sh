#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
for part in attiny25 attiny45 attiny85; do
  ${AVR_CC:-avr-gcc} -mmcu="$part" -Os -Wall -Wextra -Werror \
    -o "tests/fixtures/avr/$part-cpu-smoke.elf" \
    tests/fixtures/avr/attiny-x5-cpu-smoke.c
done
