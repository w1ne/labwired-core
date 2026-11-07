#!/usr/bin/env bash
# Builds the GPIO smoke for the imported ATtiny13, 24/44/84 and 25/45/85
# profiles. The 64-byte ATtiny13 gets a smaller scratch block for its stack.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
for part in attiny13 attiny24 attiny44 attiny84 attiny25 attiny45 attiny85; do
  scratch=48
  if [ "$part" = attiny13 ]; then scratch=16; fi
  ${AVR_CC:-avr-gcc} -mmcu="$part" -Os -Wall -Wextra -Werror -DSCRATCH_LEN="$scratch" \
    -o "tests/fixtures/avr/$part-io-smoke.elf" \
    tests/fixtures/avr/attiny-portb-io-smoke.c
done
