# Board CI Matrix — Phase 4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the tier1 validation matrix self-maintaining and unbreakable: regenerate it with CI run evidence on every push to main (post-merge), block hand-edits in PRs, and retire the race-prone nightly refresh.

**Architecture:** The /validation grid renders a cell's status only if it carries a `run_url`; only the nightly stamped those, and any PR that hand-committed `tier1-matrix.json` (status-only) wiped the evidence → all-dots until the next nightly (which also lost a push race against concurrent merges). Phase 4 moves the refresh into the Core Board CI workflow as a post-merge `aggregate` job that runs the regression ratchet, regenerates the matrix stamped with the merge's `run_url`, and commits it back race-safely — but only when per-cell STATUS actually changed, so unchanged merges don't churn main. A PR guard rejects hand-edits to the generated files so the aggregate is the sole writer.

**Tech Stack:** bash, Python 3.12 (+pytest), GitHub Actions, cargo.

## Global Constraints

- Repo `labwired-core` (public). The matrix is public proof; its honesty rule (no `run_url` → no status claim) must stay intact — do NOT weaken the renderer.
- The aggregate is the SOLE writer of `docs/coverage/tier1-matrix.json` and `docs/coverage/tier1-scoreboard.md`.
- Auto-commit to main must: (a) carry `[skip ci]` so it does not re-trigger the workflow; (b) be push-race-safe (`git pull --rebase` + retry); (c) only commit when per-cell status changed (ignore `run_url`-only diffs).
- No AI/Claude references in commit messages.
- Branch `refactor/board-ci-phase4` is off latest `main`. Use `git merge`, never rebase.
- `python` exists in CI; locally use `python3`.

---

## Task 1: Status comparator + aggregate script

**Files:**
- Create: `scripts/ci/tier1_status_equal.py`
- Test: `scripts/ci/test_tier1_status_equal.py`
- Create: `scripts/ci/aggregate-tier1.sh`

**Interfaces:**
- `tier1_status_equal.py A B` exits 0 if A and B have identical per-cell `status` (ignoring `run_url`/extra keys), else exit 1.
- `aggregate-tier1.sh <run_url>` ratchets, regenerates the matrix stamped with `<run_url>`, and commits+pushes to main only when statuses changed.

- [ ] **Step 1: Write the failing comparator test**

`scripts/ci/test_tier1_status_equal.py`:
```python
import subprocess, sys, json
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "tier1_status_equal.py"

def _write(tmp, name, data):
    p = tmp / name
    p.write_text(json.dumps(data))
    return str(p)

def _run(a, b):
    return subprocess.run([sys.executable, str(SCRIPT), a, b]).returncode

def test_equal_when_only_run_url_differs(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass", "run_url": "https://x/1"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "pass", "run_url": "https://x/2"}}})
    assert _run(a, b) == 0

def test_differs_when_status_changes(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "partial"}}})
    assert _run(a, b) == 1

def test_differs_when_cell_added(tmp_path):
    a = _write(tmp_path, "a.json", {"esp32": {"adc": {"status": "pass"}}})
    b = _write(tmp_path, "b.json", {"esp32": {"adc": {"status": "pass"}, "spi": {"status": "pass"}}})
    assert _run(a, b) == 1
```

- [ ] **Step 2: Run, expect failure**

Run: `cd ~/projects/labwired-core-boardci && python3 -m pytest scripts/ci/test_tier1_status_equal.py -v`
Expected: FAIL (script missing).

- [ ] **Step 3: Write `scripts/ci/tier1_status_equal.py`**

```python
#!/usr/bin/env python3
"""Exit 0 if two tier1 matrices have identical per-cell statuses (ignoring run_url)."""
import json
import sys


def statuses(path: str) -> dict:
    d = json.loads(open(path).read())
    return {
        chip: {cls: cell.get("status") for cls, cell in row.items()}
        for chip, row in d.items()
    }


def main() -> int:
    a, b = sys.argv[1], sys.argv[2]
    return 0 if statuses(a) == statuses(b) else 1


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: Run tests green**

Run: `python3 -m pytest scripts/ci/test_tier1_status_equal.py -v`
Expected: 3 passed.

- [ ] **Step 5: Write `scripts/ci/aggregate-tier1.sh`**

```bash
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
```

- [ ] **Step 6: Make executable + commit**

```bash
cd ~/projects/labwired-core-boardci
chmod +x scripts/ci/aggregate-tier1.sh
git add scripts/ci/tier1_status_equal.py scripts/ci/test_tier1_status_equal.py scripts/ci/aggregate-tier1.sh
git update-index --chmod=+x scripts/ci/aggregate-tier1.sh
git commit -m "ci: add tier1 aggregate refresh script (status-diff, race-safe push)"
```

---

## Task 2: Wire aggregate + guard jobs; retire nightly refresh

**Files:**
- Modify: `.github/workflows/core-board-ci.yml`
- Modify: `.github/workflows/core-nightly.yml`

- [ ] **Step 1: Add the `guard` and `aggregate` jobs to `core-board-ci.yml`**

Append these two jobs at the end of the `jobs:` map in `.github/workflows/core-board-ci.yml`:
```yaml
  guard:
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Reject hand-edits to generated tier1 coverage
        run: |
          base="${{ github.event.pull_request.base.sha }}"
          changed="$(git diff --name-only "$base"...HEAD)"
          if echo "$changed" | grep -qE '^docs/coverage/(tier1-matrix\.json|tier1-scoreboard\.md)$'; then
            echo "::error::docs/coverage/tier1-matrix.json and tier1-scoreboard.md are generated by the Core Board CI aggregate (push to main); do not hand-edit. Remove them from this PR." >&2
            exit 1
          fi

  aggregate:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Install Rust
        uses: dtolnay/rust-toolchain@1.95.0
      - name: Cache dependencies
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: core-board-aggregate
          workspaces: |
            . -> target
      - name: Refresh tier1 matrix with evidence
        run: scripts/ci/aggregate-tier1.sh "${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}"
```

- [ ] **Step 2: Remove the redundant `tier1-matrix-refresh` job from `core-nightly.yml`**

Delete the entire `tier1-matrix-refresh:` job block (from its `tier1-matrix-refresh:` key through its last step) from `.github/workflows/core-nightly.yml`. Leave every other job (validation, fixtures e2e, drift check, etc.) untouched. After editing, confirm `grep -n "tier1-matrix-refresh\|tier1-matrix " .github/workflows/core-nightly.yml` returns nothing.

- [ ] **Step 3: Lint both workflows**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/core-board-ci.yml')); yaml.safe_load(open('.github/workflows/core-nightly.yml')); print('ok')"
```
Expected: `ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/core-board-ci.yml .github/workflows/core-nightly.yml
git commit -m "ci: refresh tier1 matrix in board CI aggregate on main; guard hand-edits; drop nightly refresh"
```

---

## Task 3: Cutover — verify and merge

**Files:** none (operational).

- [ ] **Step 1: Merge latest main + push branch**

```bash
cd ~/projects/labwired-core-boardci
git fetch origin main
git merge --no-edit origin/main
python3 -m pytest scripts/ci/test_tier1_status_equal.py -q   # still 3 passed
git push -u origin refactor/board-ci-phase4
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --base main --title "Board CI matrix Phase 4: self-maintaining tier1 matrix" \
  --body "Moves tier1-matrix refresh into Core Board CI as a post-merge aggregate (regenerates + stamps run_url evidence on push to main, race-safe, [skip ci], commits only when statuses change). Adds a PR guard rejecting hand-edits to docs/coverage/tier1-matrix.json + tier1-scoreboard.md. Retires the race-prone nightly tier1-matrix-refresh. Fixes the recurring all-dots /validation breakage. Phase 4 of docs/superpowers/specs/2026-06-28-board-ci-matrix-design.md."
```

- [ ] **Step 3: Verify the guard works (negative check on the PR)**

Confirm the new `guard` job passes on this PR (this PR does not edit the generated files). To prove the guard actually fails on a hand-edit, push a throwaway commit that touches `docs/coverage/tier1-scoreboard.md`, confirm `guard` goes red, then revert it:
```bash
echo "" >> docs/coverage/tier1-scoreboard.md
git commit -am "test: trip the tier1 guard (will revert)"
git push
# watch: gh pr checks  -> guard FAILS
git revert --no-edit HEAD
git push
# watch: gh pr checks  -> guard passes again
```

- [ ] **Step 4: Confirm PR green and merge**

Run: `gh pr checks` — `core-integrity`, `Core Board CI / iolink-station-l476`, and `guard` pass. Merge with a merge commit. (Do this when main is quiet — the merge triggers the `aggregate` job which commits back to main.)

- [ ] **Step 5: Observe the first post-merge aggregate run**

After merge, watch the `Core Board CI` run on main: the `aggregate` job runs the ratchet + regenerates the matrix with run_urls + commits `chore(tier1): matrix refresh with run evidence [skip ci]` (if statuses changed vs committed). Confirm:
```bash
gh run list --workflow "Core Board CI" --branch main --limit 1
git fetch origin main && git show origin/main:docs/coverage/tier1-matrix.json | grep -c run_url   # > 0
```
Expected: matrix on main now carries run_urls; `[skip ci]` commit did not trigger a second workflow run.

---

## Self-Review

- **Spec coverage:** Phase 4 = "aggregate generators (tier1 scoreboard)". Implemented as a post-merge aggregate job (Task 2) backed by a co-located script (Task 1); validation-status is left to its existing staleness gate (not broken, out of scope).
- **No-skip-to-green / honesty:** the renderer's run_url requirement is untouched; the ratchet still gates regressions before any refresh; the guard makes the aggregate the sole writer.
- **Churn control:** `tier1_status_equal.py` ensures a commit only when per-cell status changed; `[skip ci]` + race-safe push prevent loops and lost pushes (the exact failure that left the matrix broken).
- **Type/name consistency:** `aggregate-tier1.sh` calls `tier1_status_equal.py` and `generate_tier1_scoreboard.py`; workflow passes the run URL the script expects as `$1`.
- **Placeholder scan:** none.
- **Risk:** the aggregate auto-commits to main; first real run is observed in Task 3 Step 5. Logic (ratchet, status-diff, race-safe push, skip-ci) is unit-tested where possible and reasoned where CI-only.
