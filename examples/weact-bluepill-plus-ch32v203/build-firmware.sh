#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example_dir="${repo_root}/examples/weact-bluepill-plus-ch32v203"
output="${repo_root}/tests/fixtures/ch32v203c8t6-bluepill-plus-smoke.elf"

rustc \
  --edition=2021 \
  --crate-type bin \
  --target riscv32imac-unknown-none-elf \
  -C opt-level=z \
  -C panic=abort \
  -C link-arg=-T${example_dir}/link.x \
  "${example_dir}/firmware.rs" \
  -o "${output}"

readelf -h -A "${output}" | sed -E 's/[[:space:]]+$//'
