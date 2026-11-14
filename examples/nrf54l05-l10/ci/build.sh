#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
# Both exact ELF fixture images are committed; the test runs the current
# simulator build, no Nordic toolchain or external build dependency required.
cargo build -q -p labwired-cli
