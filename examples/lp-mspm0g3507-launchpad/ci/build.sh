#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"

cargo build -p firmware-lp-mspm0g3507-smoke --release --target thumbv6m-none-eabi
mkdir -p examples/lp-mspm0g3507-launchpad/build
cp target/thumbv6m-none-eabi/release/firmware-lp-mspm0g3507-smoke \
  examples/lp-mspm0g3507-launchpad/build/firmware.elf
