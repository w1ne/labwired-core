#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_dir="${repo_root}/examples/ch32v003-evt"
output="${repo_root}/tests/fixtures/ch32v003f4p6-usart-smoke.elf"

rustc \
  --edition=2021 \
  --crate-type bin \
  --target riscv32imc-unknown-none-elf \
  -C target-feature=+e,-m,-zmmul \
  -C opt-level=z \
  -C panic=abort \
  -C link-arg=-T${example_dir}/link.x \
  "${example_dir}/firmware_usart.rs" \
  -o "${output}"

readelf -h -A "${output}" | sed -E 's/[[:space:]]+$//'
