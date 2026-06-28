#!/usr/bin/env bash
# Refresh the generated coverage docs and commit them to main:
#   - docs/coverage/tier1-matrix.json + tier1-scoreboard.md  (stamped with run_url)
#   - docs/coverage/chip-conformance.md                       (regenerated from chip YAMLs)
# Commits ONLY when content changed (tier1 matrix: per-cell status, ignoring
# run_url churn; chip-conformance: any change), UNLESS "force" is given (a
# scheduled run re-stamps run_urls so evidence links stay live). The commit
# carries [skip ci]; the push is race-safe (rebase-abort + pull --rebase + retry).
set -euo pipefail
RUN_URL="${1:?usage: aggregate-tier1.sh <run_url> [force]}"
FORCE="${2:-}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

MATRIX=docs/coverage/tier1-matrix.json
SCOREBOARD=docs/coverage/tier1-scoreboard.md
CHIPCONF=docs/coverage/chip-conformance.md
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

# Regression gate (live run vs committed snapshot) — before regenerating.
cargo test --release -p labwired-cli --test tier1_matrix_ratchet -- --nocapture

# Regenerate tier1 matrix into TMP, stamping run_url into every real cell.
cargo run -p labwired-cli --release -- tier1-matrix --json-out "$TMP" --run-url "$RUN_URL"

# Regenerate chip-conformance.md in place. This is a post-merge REFRESH, not a
# gate: the test writes the doc before its regression assert, and we never set
# UPDATE_CONFORMANCE_BASELINE, so a pre-existing regression cannot be laundered
# into the baseline. A chip-conformance regression must NOT block the tier1
# matrix refresh (the visible /validation grid), so surface it as a warning.
cargo test --release -p labwired-core --test chip_conformance -- --nocapture \
  || echo "::warning::chip-conformance regressed; doc regenerated, baseline unchanged (re-baseline intentionally with UPDATE_CONFORMANCE_BASELINE=1)"

# Update tier1 matrix only on real status change (or when forced).
set +e
python3 scripts/ci/tier1_status_equal.py "$MATRIX" "$TMP"
cmp=$?
set -e
if [ "$cmp" -gt 1 ]; then
  echo "::error::tier1_status_equal.py failed (rc=$cmp)" >&2
  exit 1
fi
if [ "$FORCE" = "force" ] || [ "$cmp" -eq 1 ]; then
  mv "$TMP" "$MATRIX"
  trap - EXIT
  python3 scripts/generate_tier1_scoreboard.py
fi

git add "$MATRIX" "$SCOREBOARD" "$CHIPCONF"
if git diff --cached --quiet; then
  echo "no coverage changes"
  exit 0
fi

git config user.name "LabWired CI"
git config user.email "ci@labwired.com"
git commit -m "chore(coverage): refresh tier1 matrix + chip-conformance with run evidence [skip ci]"

for attempt in 1 2 3 4 5; do
  git rebase --abort 2>/dev/null || true
  if git pull --rebase origin main && git push origin HEAD:main; then
    echo "coverage refreshed and pushed"
    exit 0
  fi
  echo "push race; retry $attempt"
  sleep $((attempt * 3))
done
echo "::error::failed to push coverage refresh after retries" >&2
exit 1
