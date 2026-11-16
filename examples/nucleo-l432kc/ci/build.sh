#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
# Smoke ELF is committed and reproducibly rebuildable from C; no compiler
# installation is required just to execute the committed firmware.
cargo build -q -p labwired-cli
