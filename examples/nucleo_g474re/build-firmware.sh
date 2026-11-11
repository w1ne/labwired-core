#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cargo build \
  --manifest-path "$script_dir/board_firmware/Cargo.toml" \
  --release \
  --target thumbv7em-none-eabi
