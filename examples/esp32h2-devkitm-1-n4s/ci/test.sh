#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo run -q -p labwired-cli -- test \
  --script examples/esp32h2-devkitm-1-n4s/uart-smoke.yaml \
  --output-dir out/boards/esp32h2-devkitm-1-n4s/uart-smoke \
  --no-uart-stdout
