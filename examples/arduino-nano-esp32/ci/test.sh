#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
examples/arduino-nano-esp32/verify-fixture.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-esp32/io-smoke.yaml \
  --output-dir out/boards/arduino-nano-esp32/gpio-smoke \
  --no-uart-stdout
