#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c61-devkitc-1-n8r2/uart-smoke.yaml \
  --output-dir out/boards/esp32c61-devkitc-1-n8r2/uart-smoke \
  --no-uart-stdout
