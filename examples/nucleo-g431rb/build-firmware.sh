#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
firmware_dir="${repo_root}/examples/nucleo-g431rb/board_firmware"

CARGO_TARGET_DIR="${firmware_dir}/target" cargo build \
  --manifest-path "${firmware_dir}/Cargo.toml" \
  --release \
  --target thumbv7em-none-eabihf
