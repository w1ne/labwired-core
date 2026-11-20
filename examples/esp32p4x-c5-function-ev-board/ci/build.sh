#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"
cargo build -p firmware-esp32p4-demo --release --target riscv32imc-unknown-none-elf
