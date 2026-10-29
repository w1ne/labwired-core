#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
${AVR_CC:-avr-gcc} -mmcu=atmega1281 -DF_CPU=16000000UL -Os -Wall -Wextra -Werror \
  -o tests/fixtures/avr/atmega1281-io.elf \
  examples/atmega1281-io/src/main.c
cargo build -q -p labwired-cli
