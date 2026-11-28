#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "${script_dir}" rev-parse --show-toplevel)"
cd "${repo_root}"

# This committed firmware_survival fixture must follow the source because the
# RA4M1 smoke now clears MSTPCRB bit 29 before writing SCI2.
cargo build --locked -p firmware-ra4m1-demo --release --target thumbv7em-none-eabi
install -m 0755 \
  target/thumbv7em-none-eabi/release/firmware-ra4m1-demo \
  tests/fixtures/ra4m1-uno-r4-smoke.elf
