#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cargo build \
  --manifest-path "${repo_root}/examples/xiao-esp32c6-blinky/firmware/Cargo.toml" \
  --release \
  --target-dir "${repo_root}/target" \
  --target riscv32imc-unknown-none-elf
