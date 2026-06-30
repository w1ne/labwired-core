# Board CI Matrix — Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the three nightly firmware boards (`ci-fixture-arm`, `ci-fixture-riscv`, `nucleo-h563zi`) onto the manifest-driven Core Board CI, and generalize the toolchain model so heterogeneous boards are expressible.

**Architecture:** Phase 1 used an opaque `toolchains: [name]` field mapped to hardcoded install branches in the workflow. That does not express the real needs (these boards need specific rustup targets; the iolink board needs an apt package). Phase 2 replaces `toolchains` with two explicit fields — `apt` (apt packages) and `rust_targets` (rustup targets) — emitted into the matrix and consumed by generic install steps. Each board gets a home dir with co-located `ci/build.sh` + `ci/test.sh`.

**Tech Stack:** Python 3.12 + pytest (manifest tooling), GitHub Actions matrix, bash.

## Global Constraints

- Repo: `labwired-core` (public). Gate jobs stay green — public proof.
- A broken/missing firmware build must FAIL — never skip to green. Each board's audit step keeps `--fail-on-unsupported`.
- The existing `iolink-station-l476` firmware-gate must remain green after the schema change (it becomes `apt: [gcc-arm-none-eabi]`).
- The three migrated boards stay nightly: `gate: false` (run on schedule/workflow_dispatch, NOT on PR/push).
- No AI/Claude references in commit messages.
- Branch `refactor/board-ci-phase2` is off latest `main` (which has Phase 1). Use `git merge`, never rebase.
- `python` exists in CI (setup-python); locally use `python3`.

---

## Task 1: Generalize toolchain schema (apt + rust_targets)

**Files:**
- Modify: `scripts/ci/board_matrix.py`
- Modify: `scripts/ci/test_board_matrix.py`
- Modify: `configs/ci/boards.yml`

**Interfaces:**
- `to_matrix` stops emitting `toolchains`; emits `apt` (space-joined `apt` list) and `rust_targets` (space-joined `rust_targets` list). All other keys unchanged (`id,kind,path,packs,submodules`).

- [ ] **Step 1: Update the failing tests first**

In `scripts/ci/test_board_matrix.py`, change the `_entry` helper to use the new fields and update the matrix assertion. Replace the `toolchains=["arm-none-eabi"]` line in `_entry`'s `base` dict with:
```python
        apt=["gcc-arm-none-eabi"],
        rust_targets=[],
```
Replace `test_to_matrix_joins_lists_to_strings` with:
```python
def test_to_matrix_joins_lists_to_strings():
    m = bm.to_matrix([_entry(apt=["gcc-arm-none-eabi"], rust_targets=["thumbv6m-none-eabi"], packs=["stm32cubel4@v1.18.2"])])
    inc = m["include"][0]
    assert inc["apt"] == "gcc-arm-none-eabi"
    assert inc["rust_targets"] == "thumbv6m-none-eabi"
    assert inc["packs"] == "stm32cubel4@v1.18.2"
    assert "toolchains" not in inc
```

- [ ] **Step 2: Run tests, expect failure**

Run: `cd ~/projects/labwired-core-boardci && python3 -m pytest scripts/ci/test_board_matrix.py -v -k to_matrix`
Expected: FAIL (`to_matrix` still emits `toolchains`, not `apt`/`rust_targets`).

- [ ] **Step 3: Update `to_matrix` in `scripts/ci/board_matrix.py`**

Replace the `include.append({...})` block in `to_matrix` with:
```python
        include.append({
            "id": e["id"],
            "kind": e["kind"],
            "path": e["path"],
            "apt": " ".join(e.get("apt", [])),
            "rust_targets": " ".join(e.get("rust_targets", [])),
            "packs": " ".join(e.get("packs", [])),
            "submodules": e.get("submodules", "false"),
        })
```

- [ ] **Step 4: Update the iolink entry in `configs/ci/boards.yml`**

Replace its `toolchains: [arm-none-eabi]` line with:
```yaml
    apt: [gcc-arm-none-eabi]
```
(iolink builds C via `make`/`arm-none-eabi-gcc`; it needs no rustup target.) Also update the header comment block: replace the `# toolchains: ...` line with:
```yaml
# apt:         apt packages to install (e.g. gcc-arm-none-eabi)
# rust_targets: rustup targets to add (e.g. thumbv6m-none-eabi)
```

- [ ] **Step 5: Run the full suite green**

Run: `python3 -m pytest scripts/ci/test_board_matrix.py -v`
Expected: all pass (8/8 — the real iolink entry now validates with the new fields; `validate()` doesn't inspect `toolchains`/`apt`/`rust_targets`, only `kind`/`path`/scripts, so it is unaffected).

- [ ] **Step 6: Commit**

```bash
git add scripts/ci/board_matrix.py scripts/ci/test_board_matrix.py configs/ci/boards.yml
git commit -m "ci: generalize board toolchains to explicit apt + rust_targets"
```

---

## Task 2: Generic toolchain install steps in the workflow

**Files:**
- Modify: `.github/workflows/core-board-ci.yml`

**Interfaces:**
- Consumes `matrix.apt` and `matrix.rust_targets` (space-joined strings) instead of `matrix.toolchains`.

- [ ] **Step 1: Replace the toolchain install steps**

In `.github/workflows/core-board-ci.yml`, delete the two existing steps "Install ARM bare-metal toolchain" (the `contains(matrix.toolchains, 'arm-none-eabi')` one) and "Install RISC-V target" (the `contains(matrix.toolchains, 'riscv')` one). Replace them with these two generic steps (place them where the old ones were, after "Cache dependencies" and before "Fetch device packs"):
```yaml
      - name: Install apt packages
        if: matrix.apt != ''
        env:
          APT: ${{ matrix.apt }}
        run: |
          sudo apt-get update
          sudo apt-get install -y $APT

      - name: Add rust targets
        if: matrix.rust_targets != ''
        env:
          RUST_TARGETS: ${{ matrix.rust_targets }}
        run: |
          for t in $RUST_TARGETS; do rustup target add "$t"; done
```

- [ ] **Step 2: Lint the workflow**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/core-board-ci.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/core-board-ci.yml
git commit -m "ci: install toolchains from matrix apt + rust_targets"
```

---

## Task 3: Co-located scripts + manifest entries for the 3 nightly boards

**Files:**
- Create: `examples/ci-fixture-arm/ci/build.sh`, `examples/ci-fixture-arm/ci/test.sh`
- Create: `examples/ci-fixture-riscv/ci/build.sh`, `examples/ci-fixture-riscv/ci/test.sh`
- Create: `examples/nucleo-h563zi/ci/build.sh`, `examples/nucleo-h563zi/ci/test.sh`
- Modify: `configs/ci/boards.yml`

**Interfaces:** consumes nothing new; each `test.sh` appends a metrics summary to `$GITHUB_STEP_SUMMARY` only when that env var is set.

- [ ] **Step 1: ci-fixture-arm scripts**

`examples/ci-fixture-arm/ci/build.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-ci-fixture --release --target thumbv6m-none-eabi
```
`examples/ci-fixture-arm/ci/test.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ci-fixture-arm
cargo run -q -p labwired-cli -- test --script examples/ci/uart-ok.yaml --output-dir "$OUT/smoke" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/ci/dummy-max-uart-bytes.yaml --output-dir "$OUT/max-uart" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/ci/dummy-no-progress.yaml --output-dir "$OUT/no-progress" --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv6m-none-eabi/release/firmware-ci-fixture \
  --system configs/systems/ci-fixture-uart1.yaml \
  --max-steps 5000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  python3 - "$OUT/unsupported-audit/metrics.json" "CI Fixture ARM" >> "$GITHUB_STEP_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
m = json.loads(Path(sys.argv[1]).read_text())
print(f"### {sys.argv[2]} Instruction Support\n")
print(f"- Instructions executed: `{m['instructions_executed']}`")
print(f"- Unsupported observations: `{m['unsupported_total']}`")
print(f"- Instruction support coverage: `{m['instruction_support_percent']}%`")
PY
fi
```

- [ ] **Step 2: ci-fixture-riscv scripts**

`examples/ci-fixture-riscv/ci/build.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p riscv-ci-fixture --release --target riscv32i-unknown-none-elf
```
`examples/ci-fixture-riscv/ci/test.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ci-fixture-riscv
cargo run -q -p labwired-cli -- test --script examples/ci/riscv-uart-ok.yaml --output-dir "$OUT/smoke" --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32i-unknown-none-elf/release/riscv-ci-fixture \
  --system configs/systems/ci-fixture-riscv-uart1.yaml \
  --max-steps 5000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  python3 - "$OUT/unsupported-audit/metrics.json" "CI Fixture RISC-V" >> "$GITHUB_STEP_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
m = json.loads(Path(sys.argv[1]).read_text())
print(f"### {sys.argv[2]} Instruction Support\n")
print(f"- Instructions executed: `{m['instructions_executed']}`")
print(f"- Unsupported observations: `{m['unsupported_total']}`")
print(f"- Instruction support coverage: `{m['instruction_support_percent']}%`")
PY
fi
```

- [ ] **Step 3: nucleo-h563zi scripts**

`examples/nucleo-h563zi/ci/build.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
cargo build -p firmware-h563-fullchip-demo --release --target thumbv7m-none-eabi
```
`examples/nucleo-h563zi/ci/test.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-h563zi
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml --output-dir "$OUT/io-smoke" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/fullchip-smoke.yaml --output-dir "$OUT/fullchip-smoke" --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-h563-io-demo \
  --system configs/systems/nucleo-h563zi-demo.yaml \
  --max-steps 20000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  python3 - "$OUT/unsupported-audit/metrics.json" "NUCLEO-H563ZI" >> "$GITHUB_STEP_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
m = json.loads(Path(sys.argv[1]).read_text())
print(f"### {sys.argv[2]} Instruction Support\n")
print(f"- Instructions executed: `{m['instructions_executed']}`")
print(f"- Unsupported observations: `{m['unsupported_total']}`")
print(f"- Instruction support coverage: `{m['instruction_support_percent']}%`")
PY
fi
```

- [ ] **Step 4: Make scripts executable (committed bit)**

```bash
cd ~/projects/labwired-core-boardci
chmod +x examples/ci-fixture-arm/ci/*.sh examples/ci-fixture-riscv/ci/*.sh examples/nucleo-h563zi/ci/*.sh
git add examples/ci-fixture-arm/ci examples/ci-fixture-riscv/ci examples/nucleo-h563zi/ci
git update-index --chmod=+x examples/ci-fixture-arm/ci/build.sh examples/ci-fixture-arm/ci/test.sh \
  examples/ci-fixture-riscv/ci/build.sh examples/ci-fixture-riscv/ci/test.sh \
  examples/nucleo-h563zi/ci/build.sh examples/nucleo-h563zi/ci/test.sh
```

- [ ] **Step 5: Add the 3 manifest entries to `configs/ci/boards.yml`** (under `boards:`, after the iolink entry)

```yaml
  - id: ci-fixture-arm
    kind: firmware-gate
    path: examples/ci-fixture-arm
    rust_targets: [thumbv6m-none-eabi]
    gate: false
  - id: ci-fixture-riscv
    kind: firmware-gate
    path: examples/ci-fixture-riscv
    rust_targets: [riscv32i-unknown-none-elf]
    gate: false
  - id: nucleo-h563zi
    kind: firmware-gate
    path: examples/nucleo-h563zi
    rust_targets: [thumbv7m-none-eabi]
    gate: false
```

- [ ] **Step 6: Validate the manifest + confirm matrix selection**

Run:
```bash
python3 -m pytest scripts/ci/test_board_matrix.py -q   # 8/8 (validator sees new dirs+scripts)
python3 scripts/ci/board_matrix.py --event pull_request | python3 -m json.tool   # only iolink (gate:true)
python3 scripts/ci/board_matrix.py --event schedule | python3 -c "import sys,json; print(sorted(e['id'] for e in json.load(sys.stdin)['include']))"
```
Expected: pytest 8/8; PR matrix has only `iolink-station-l476`; schedule matrix lists all four ids.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "ci: migrate ci-fixture-arm, ci-fixture-riscv, nucleo-h563zi to board manifest"
```

---

## Task 4: Verify all boards build, then remove old YAMLs

**Files:**
- Delete: `.github/workflows/core-board-ci-fixture-arm.yml`
- Delete: `.github/workflows/core-board-ci-fixture-riscv.yml`
- Delete: `.github/workflows/core-board-nucleo-h563zi.yml`

- [ ] **Step 1: Push the branch**

```bash
git push -u origin refactor/board-ci-phase2
```

- [ ] **Step 2: Trigger a full (all-entries) run via workflow_dispatch on the branch**

The nightly boards are `gate:false`, so they do NOT run on the PR. A `workflow_dispatch` event makes `select()` return ALL entries:
```bash
gh workflow run "Core Board CI" --ref refactor/board-ci-phase2
sleep 5
gh run list --workflow "Core Board CI" --branch refactor/board-ci-phase2 --limit 1
```

- [ ] **Step 3: Watch the dispatched run; all four board jobs must pass**

Run: `gh run watch <run-id>` (or poll `gh run view <run-id> --json jobs`).
Expected: jobs `iolink-station-l476`, `ci-fixture-arm`, `ci-fixture-riscv`, `nucleo-h563zi` all `success`. Do NOT proceed until all four are green. If a board fails, fix its script/manifest entry and re-dispatch.

- [ ] **Step 4: Open the PR**

```bash
gh pr create --base main --title "Board CI matrix Phase 2: migrate nightly firmware boards" \
  --body "Migrates ci-fixture-arm, ci-fixture-riscv, nucleo-h563zi onto the manifest-driven Core Board CI (gate:false, nightly). Generalizes the toolchain model to explicit apt + rust_targets. Phase 2 of docs/superpowers/specs/2026-06-28-board-ci-matrix-design.md. All four boards verified green via workflow_dispatch (run linked in comments)."
```

- [ ] **Step 5: Delete the three superseded YAMLs**

```bash
git rm .github/workflows/core-board-ci-fixture-arm.yml \
       .github/workflows/core-board-ci-fixture-riscv.yml \
       .github/workflows/core-board-nucleo-h563zi.yml
git commit -m "ci: remove per-board nightly YAMLs, superseded by Core Board CI"
git push
```

- [ ] **Step 6: Confirm PR green (core-integrity + iolink gate) and merge**

Run: `gh pr checks` — `core-integrity` and `Core Board CI / iolink-station-l476` pass (nightly boards don't run on the PR; they were verified via dispatch in Step 3).
Then merge with a merge commit.

---

## Self-Review

- **Spec coverage:** Phase 2 = "fold in the 3 nightly firmware boards (gate:false)" — Tasks 1-3 add the entries + scripts; Task 4 verifies and removes old YAMLs. The toolchain generalization (apt + rust_targets) is the enabling refactor the migration requires.
- **No-skip-to-green:** every board's `test.sh` keeps `--fail-on-unsupported`; `set -euo pipefail` aborts on any failed build/smoke step; the manifest validator still fails fast on missing scripts.
- **Type/name consistency:** `to_matrix` emits `apt`/`rust_targets`/`packs`/`submodules`/`id`/`kind`/`path`; the workflow's install steps read `matrix.apt`/`matrix.rust_targets`; `toolchains` is fully removed from manifest, emitter, and workflow.
- **Placeholder scan:** none — all scripts and entries are complete.
- **Verification gap:** nightly boards can't be verified on the PR (gate:false). Task 4 Step 2-3 closes this with a `workflow_dispatch` full run before old YAMLs are deleted.
