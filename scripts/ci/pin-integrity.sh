#!/usr/bin/env bash
# Canonical "is this core checkout healthy" checks — one source of truth so
# consumers (e.g. the app repo's pinned-submodule integrity gate) reference
# these instead of re-hardcoding the commands in their own workflows.
#   smoke        — run the CLI against a deterministic CI script
#   determinism  — prove repeated runs match (result + trace)
#   all          — both (default)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

run_smoke() {
  cargo run -q -p labwired-cli -- test \
    --script examples/ci/dummy-max-uart-bytes.yaml \
    --output-dir out/integration-smoke --no-uart-stdout
}
run_determinism() {
  cargo test -p labwired-cli --test determinism -- --nocapture
}

case "${1:-all}" in
  smoke) run_smoke ;;
  determinism) run_determinism ;;
  all) run_smoke; run_determinism ;;
  *) echo "usage: pin-integrity.sh [smoke|determinism|all]" >&2; exit 1 ;;
esac
