#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
examples/adafruit-feather-esp32-v2/verify-fixture.sh
cargo run -q -p labwired-cli -- test \
  --script examples/adafruit-feather-esp32-v2/io-smoke.yaml \
  --output-dir out/boards/adafruit-feather-esp32-v2/gpio-smoke \
  --no-uart-stdout
