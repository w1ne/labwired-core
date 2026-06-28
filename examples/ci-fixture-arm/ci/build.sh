#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-ci-fixture --release --target thumbv6m-none-eabi
