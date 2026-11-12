#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo build -p firmware-esp32c5-demo --release --target riscv32imc-unknown-none-elf
