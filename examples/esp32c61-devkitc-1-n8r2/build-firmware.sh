#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo build \
  --manifest-path "${repo_root}/crates/firmware-esp32c61-smoke/Cargo.toml" \
  --target riscv32imc-unknown-none-elf \
  --release
