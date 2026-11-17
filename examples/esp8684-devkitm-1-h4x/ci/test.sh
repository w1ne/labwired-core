#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo run -q -p labwired-cli -- test \
  --script examples/esp8684-devkitm-1-h4x/uart-smoke.yaml \
  --output-dir out/boards/esp8684-devkitm-1-h4x/uart-smoke \
  --no-uart-stdout
