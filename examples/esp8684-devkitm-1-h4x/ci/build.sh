#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo build --release --target riscv32imac-unknown-none-elf \
  -p firmware-esp8684-devkitm-1-h4x-smoke
