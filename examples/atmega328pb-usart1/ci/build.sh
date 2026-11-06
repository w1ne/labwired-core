#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
mkdir -p tests/fixtures/avr
${AVR_CC:-avr-gcc} -mmcu=atmega328pb -nostartfiles -nodevicelib \
  -DF_CPU=16000000UL -Os -Wall -Wextra -Werror -Wl,-e,__vectors \
  -o tests/fixtures/avr/atmega328pb-usart1.elf \
  examples/atmega328pb-usart1/src/main.c
cargo build -q -p labwired-cli
