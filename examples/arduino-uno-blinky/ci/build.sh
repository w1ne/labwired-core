#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -q -p labwired-cli
test -s tests/fixtures/avr/arduino-uno-blinky.elf
test -s tests/fixtures/avr/arduino-uno-io.elf
