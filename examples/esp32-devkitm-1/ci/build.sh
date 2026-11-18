#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "${root}"
cargo build -q -p labwired-cli
