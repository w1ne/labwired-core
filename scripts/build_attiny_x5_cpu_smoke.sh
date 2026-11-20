#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
for part in attiny13 attiny2313 attiny2313a attiny4313 attiny24 attiny44 attiny84 \
  attiny25 attiny45 attiny85 attiny1634; do
  ${AVR_CC:-avr-gcc} -mmcu="$part" -Os -Wall -Wextra -Werror \
    -o "tests/fixtures/avr/$part-cpu-smoke.elf" \
    tests/fixtures/avr/attiny-x5-cpu-smoke.c
done
