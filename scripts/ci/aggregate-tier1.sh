#!/usr/bin/env bash
# Regenerate docs/coverage/tier1-matrix.json with CI run evidence and commit to
# main — but only when per-cell STATUS changed (not merely the run_url), so
# unchanged merges do not churn main. Push is race-safe (rebase + retry). The
# commit carries [skip ci] so it does not re-trigger the workflow.
set -euo pipefail
RUN_URL="${1:?usage: aggregate-tier1.sh <run_url>}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

MATRIX=docs/coverage/tier1-matrix.json
SCOREBOARD=docs/coverage/tier1-scoreboard.md
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

# Regression gate: a live run must not silently regress the committed snapshot.
cargo test --release -p labwired-cli --test tier1_matrix_ratchet -- --nocapture

# Regenerate, stamping the run_url into every real (non-na/unrecorded) cell.
cargo run -p labwired-cli --release -- tier1-matrix --json-out "$TMP" --run-url "$RUN_URL"

# Skip if statuses are unchanged (avoid run_url-only churn).
if python3 scripts/ci/tier1_status_equal.py "$MATRIX" "$TMP"; then
  echo "tier1 statuses unchanged; no refresh needed"
  exit 0
fi

mv "$TMP" "$MATRIX"
trap - EXIT
python3 scripts/generate_tier1_scoreboard.py

git config user.name "LabWired CI"
git config user.email "ci@labwired.com"
git add "$MATRIX" "$SCOREBOARD"
git commit -m "chore(tier1): matrix refresh with run evidence [skip ci]"

for attempt in 1 2 3 4 5; do
  if git pull --rebase origin main && git push origin HEAD:main; then
    echo "tier1 matrix refreshed and pushed"
    exit 0
  fi
  echo "push race; retry $attempt"
  sleep $((attempt * 3))
done
echo "::error::failed to push tier1 refresh after retries" >&2
exit 1
