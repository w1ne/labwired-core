#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p riscv-ci-fixture --release --target riscv32i-unknown-none-elf
