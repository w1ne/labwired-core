#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
avr-gcc -mmcu=atmega1281 -DF_CPU=16000000UL -Os -Wall -Wextra -Werror \
  -o tests/fixtures/avr/stk600-atmega1281.elf \
  examples/stk600-atmega1281/src/main.c
cargo build -q -p labwired-cli
