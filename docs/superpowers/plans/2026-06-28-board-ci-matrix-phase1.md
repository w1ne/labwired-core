# Board CI Matrix — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-written `core-iolink-station.yml` with a manifest-driven board-CI mechanism, migrating the `iolink-station-l476` gate onto it with its check name preserved.

**Architecture:** A central `configs/ci/boards.yml` holds policy/metadata per board. A Python module (`scripts/ci/board_matrix.py`) validates the manifest, filters entries by CI event, and emits a GitHub Actions matrix. A single workflow (`core-board-ci.yml`) runs a `setup` job (validate + emit matrix) then a `board` job (per-entry build/test via co-located scripts). Board-specific build logic moves into `examples/iolink-station/ci/{build,test}.sh`; the STM32CubeL4 fetch becomes a reusable composite action.

**Tech Stack:** Python 3.12 + pytest 9 (manifest tooling), PyYAML, GitHub Actions (composite action + matrix), bash.

## Global Constraints

- Repo: `labwired-core` (public). Gate jobs stay in core and stay green — public proof.
- The migrated gate MUST keep the exact check name `iolink-station-l476` (branch-protection required-check).
- A broken/missing firmware build must FAIL the gate, never skip to green: keep `LABWIRED_REQUIRE_IOLINK_ELFS=1` and the explicit `test -f <elf>` guards.
- Commit author/email per repo convention; no AI/Claude references in commit messages.
- Branch `refactor/board-ci-matrix` is off `origin/main`. **Precondition:** PR #393 (iolinki → v1.1.3, adds `frame.c`) must be merged to `main` and merged into this branch first, or the iolink build is red for an unrelated reason. Use `git merge`, never rebase.

---

## Precondition Task: rebase-free sync of the Phase 0 fix

- [ ] **Step 1: Confirm #393 is merged to main, then merge main into this branch**

Run:
```bash
cd ~/projects/labwired-core-boardci
gh pr view 393 --json state -q .state   # expect MERGED before proceeding
git fetch origin main
git merge --no-edit origin/main
git ls-tree HEAD third_party/iolinki     # pin should be v1.1.3 (8381cd0b...), not f2c07ef
```
Expected: merge clean; submodule pin advanced to the v1.1.3 commit.

If #393 is not yet merged, stop and merge it first; the gate cannot pass without `frame.c`.

---

## Task 1: Manifest + matrix tooling (Python, TDD)

**Files:**
- Create: `configs/ci/boards.yml`
- Create: `scripts/ci/board_matrix.py`
- Test: `scripts/ci/test_board_matrix.py`

**Interfaces:**
- Consumes: nothing (entry point).
- Produces:
  - `load_manifest(path: str) -> list[dict]`
  - `validate(entries: list[dict], repo_root: str) -> list[str]` — returns human-readable error strings; empty list == valid.
  - `select(entries: list[dict], event: str) -> list[dict]` — `event` in {`pull_request`,`push`,`schedule`,`workflow_dispatch`}; PR/push → only `gate == True`; schedule/workflow_dispatch → all.
  - `to_matrix(entries: list[dict]) -> dict` — `{"include": [ {id,kind,path,toolchains,packs,submodules}, ... ]}` (lists joined to space-separated strings for shell consumption: `toolchains` and `packs` become strings).
  - CLI: `python scripts/ci/board_matrix.py --event <event> [--repo-root .]` prints matrix JSON to stdout; exits non-zero with errors on stderr if validation fails.

- [ ] **Step 1: Write the failing tests**

Create `scripts/ci/test_board_matrix.py`:
```python
import json
import subprocess
import sys
from pathlib import Path

import board_matrix as bm

REPO_ROOT = Path(__file__).resolve().parents[2]


def _entry(**over):
    base = dict(
        id="demo-board",
        kind="firmware-gate",
        path="examples/iolink-station",
        toolchains=["arm-none-eabi"],
        packs=["stm32cubel4@v1.18.2"],
        submodules="recursive",
        gate=True,
    )
    base.update(over)
    return base


def test_select_pull_request_keeps_only_gates():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "pull_request")] == ["a"]


def test_select_schedule_keeps_all():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "schedule")] == ["a", "b"]


def test_validate_flags_missing_build_script():
    bad = _entry(path="examples/does-not-exist")
    errors = bm.validate([bad], str(REPO_ROOT))
    assert any("ci/build.sh" in e for e in errors)


def test_validate_passes_for_real_iolink_entry():
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    assert bm.validate(entries, str(REPO_ROOT)) == []


def test_to_matrix_joins_lists_to_strings():
    m = bm.to_matrix([_entry()])
    inc = m["include"][0]
    assert inc["toolchains"] == "arm-none-eabi"
    assert inc["packs"] == "stm32cubel4@v1.18.2"


def test_cli_emits_json_for_pull_request():
    out = subprocess.check_output(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT)],
        text=True,
    )
    matrix = json.loads(out)
    assert any(e["id"] == "iolink-station-l476" for e in matrix["include"])
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/projects/labwired-core-boardci && python -m pytest scripts/ci/test_board_matrix.py -v`
Expected: collection/import error or FAIL — `board_matrix` and `configs/ci/boards.yml` do not exist yet.

- [ ] **Step 3: Create the manifest**

Create `configs/ci/boards.yml`:
```yaml
# Board/example CI manifest. One entry per CI target.
# kind:        firmware-gate | sim-validate | aggregate
# gate: true   -> runs on pull_request/push to main; false -> nightly only
# path:        directory whose ci/build.sh + ci/test.sh run the entry
# toolchains:  toolchains the workflow installs (e.g. arm-none-eabi, riscv)
# packs:       named, reusable device-pack fetchers (see .github/actions/fetch-pack)
boards:
  - id: iolink-station-l476
    kind: firmware-gate
    path: examples/iolink-station
    toolchains: [arm-none-eabi]
    packs: [stm32cubel4@v1.18.2]
    submodules: recursive
    gate: true
```

- [ ] **Step 4: Implement `board_matrix.py`**

Create `scripts/ci/board_matrix.py`:
```python
#!/usr/bin/env python3
"""Validate the board CI manifest and emit a GitHub Actions matrix."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import yaml

GATE_EVENTS = {"pull_request", "push"}
ALL_EVENTS = {"schedule", "workflow_dispatch"}


def load_manifest(path: str) -> list[dict]:
    data = yaml.safe_load(Path(path).read_text()) or {}
    return list(data.get("boards", []))


def validate(entries: list[dict], repo_root: str) -> list[str]:
    root = Path(repo_root)
    errors: list[str] = []
    seen: set[str] = set()
    for e in entries:
        eid = e.get("id", "<no-id>")
        if eid in seen:
            errors.append(f"{eid}: duplicate id")
        seen.add(eid)
        kind = e.get("kind")
        if kind not in {"firmware-gate", "sim-validate", "aggregate"}:
            errors.append(f"{eid}: invalid kind {kind!r}")
        path = root / e.get("path", "")
        if not path.is_dir():
            errors.append(f"{eid}: path {e.get('path')!r} is not a directory")
            continue
        if kind == "firmware-gate":
            for script in ("ci/build.sh", "ci/test.sh"):
                if not (path / script).is_file():
                    errors.append(f"{eid}: missing {e['path']}/{script}")
    return errors


def select(entries: list[dict], event: str) -> list[dict]:
    if event in GATE_EVENTS:
        return [e for e in entries if e.get("gate")]
    return list(entries)


def to_matrix(entries: list[dict]) -> dict:
    include = []
    for e in entries:
        include.append({
            "id": e["id"],
            "kind": e["kind"],
            "path": e["path"],
            "toolchains": " ".join(e.get("toolchains", [])),
            "packs": " ".join(e.get("packs", [])),
            "submodules": e.get("submodules", "false"),
        })
    return {"include": include}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--event", required=True)
    ap.add_argument("--repo-root", default=".")
    ap.add_argument("--manifest", default="configs/ci/boards.yml")
    args = ap.parse_args()

    manifest = str(Path(args.repo_root) / args.manifest)
    entries = load_manifest(manifest)
    errors = validate(entries, args.repo_root)
    if errors:
        print("Manifest validation failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1
    print(json.dumps(to_matrix(select(entries, args.event))))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

Note: the tests pass `examples/iolink-station` as the path, which must contain
`ci/build.sh` + `ci/test.sh` for `test_validate_passes_for_real_iolink_entry` to
pass. Those scripts are created in Task 3, so run the full suite green only after
Task 3. Until then, expect that one test to fail and the rest to pass.

- [ ] **Step 5: Run the event/validation/matrix tests (subset)**

Run: `cd ~/projects/labwired-core-boardci && python -m pytest scripts/ci/test_board_matrix.py -v -k "not real_iolink"`
Expected: PASS for select/validate-missing/to_matrix/cli tests.

- [ ] **Step 6: Commit**

```bash
git add configs/ci/boards.yml scripts/ci/board_matrix.py scripts/ci/test_board_matrix.py
git commit -m "ci: add board CI manifest + matrix emitter"
```

---

## Task 2: `fetch-pack` composite action

**Files:**
- Create: `.github/actions/fetch-pack/action.yml`

**Interfaces:**
- Consumes: input `pack` (e.g. `stm32cubel4@v1.18.2`).
- Produces: exports `STM32CUBE_L4_DIR` to `$GITHUB_ENV` (consumed by Task 3's build.sh).

- [ ] **Step 1: Write the action**

Create `.github/actions/fetch-pack/action.yml` (logic lifted verbatim from the
sparse-clone block in the old `core-iolink-station.yml`):
```yaml
name: Fetch device pack
description: Sparse-fetch a named CMSIS/vendor device pack and export its dir to env.
inputs:
  pack:
    description: "pack id, e.g. stm32cubel4@v1.18.2"
    required: true
runs:
  using: composite
  steps:
    - name: Fetch pack
      shell: bash
      run: |
        set -euo pipefail
        name="${{ inputs.pack }}"
        case "$name" in
          stm32cubel4@*)
            tag="${name#*@}"
            CUBE="$RUNNER_TEMP/STM32CubeL4"
            git clone --depth 1 --branch "$tag" \
              --filter=blob:none --sparse \
              https://github.com/STMicroelectronics/STM32CubeL4.git "$CUBE"
            git -C "$CUBE" sparse-checkout set \
              Drivers/CMSIS/Include \
              Drivers/CMSIS/Device/ST \
              Projects/NUCLEO-L476RG/Templates/STM32CubeIDE
            git -C "$CUBE" submodule update --init --depth 1 \
              Drivers/CMSIS/Device/ST/STM32L4xx
            echo "STM32CUBE_L4_DIR=$CUBE" >> "$GITHUB_ENV"
            ;;
          *)
            echo "Unknown pack: $name" >&2
            exit 1
            ;;
        esac
```

- [ ] **Step 2: Lint the YAML**

Run: `python -c "import yaml,sys; yaml.safe_load(open('.github/actions/fetch-pack/action.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add .github/actions/fetch-pack/action.yml
git commit -m "ci: add reusable fetch-pack composite action"
```

---

## Task 3: Co-located iolink build/test scripts

**Files:**
- Create: `examples/iolink-station/ci/build.sh`
- Create: `examples/iolink-station/ci/test.sh`

**Interfaces:**
- Consumes: `STM32CUBE_L4_DIR` env (from Task 2's action).
- Produces: three ELFs and runs `world_multichip`; non-zero exit on any failure.

- [ ] **Step 1: Write `build.sh`** (build commands + ELF guards lifted from old YAML)

Create `examples/iolink-station/ci/build.sh`:
```bash
#!/usr/bin/env bash
# Build the three STM32L476 IO-Link station firmwares (real stacks + CubeL4).
set -euo pipefail
: "${STM32CUBE_L4_DIR:?STM32CUBE_L4_DIR must be set (fetch-pack action)}"
ROOT="$(git rev-parse --show-toplevel)"

make -C "$ROOT/examples/iolink-dido/firmware"           STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/master-fw"        STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/master-fw-4port"  STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"

# Hard-fail if any ELF is missing, so a silent build miss can't slip the gate.
test -f "$ROOT/examples/iolink-dido/firmware/iolink_dido.elf"
test -f "$ROOT/examples/iolink-station/master-fw/master.elf"
test -f "$ROOT/examples/iolink-station/master-fw-4port/master.elf"
```

- [ ] **Step 2: Write `test.sh`**

Create `examples/iolink-station/ci/test.sh`:
```bash
#!/usr/bin/env bash
# Run the multi-chip station integration tests against the freshly built ELFs.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
export LABWIRED_REQUIRE_IOLINK_ELFS=1
cargo test -p labwired-core --release --test world_multichip -- --nocapture
```

- [ ] **Step 3: Make them executable + commit**

```bash
chmod +x examples/iolink-station/ci/build.sh examples/iolink-station/ci/test.sh
git add examples/iolink-station/ci/build.sh examples/iolink-station/ci/test.sh
git update-index --chmod=+x examples/iolink-station/ci/build.sh examples/iolink-station/ci/test.sh
git commit -m "ci: co-locate iolink-station build/test scripts"
```

- [ ] **Step 4: Now the full manifest tooling suite passes**

Run: `python -m pytest scripts/ci/test_board_matrix.py -v`
Expected: ALL PASS (including `test_validate_passes_for_real_iolink_entry`, now that the scripts exist).

- [ ] **Step 5: Local smoke (optional, requires arm toolchain + Cube pack)**

If `arm-none-eabi-gcc` and a local STM32CubeL4 checkout are available:
```bash
STM32CUBE_L4_DIR=/path/to/STM32CubeL4 examples/iolink-station/ci/build.sh && \
examples/iolink-station/ci/test.sh
```
Expected: three ELFs built, `world_multichip` passes. Skip if toolchain absent; CI is the authoritative check.

---

## Task 4: `core-board-ci.yml` workflow

**Files:**
- Create: `.github/workflows/core-board-ci.yml`

**Interfaces:**
- Consumes: `scripts/ci/board_matrix.py`, `.github/actions/fetch-pack`, `configs/ci/boards.yml`, each entry's `ci/build.sh` + `ci/test.sh`.
- Produces: one check per matrix entry named exactly by `matrix.id` (so `iolink-station-l476` is preserved).

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/core-board-ci.yml`:
```yaml
name: Core Board CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
  schedule:
    - cron: "0 3 * * *"
  workflow_dispatch:

jobs:
  setup:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.gen.outputs.matrix }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - run: pip install pyyaml
      - id: gen
        run: |
          echo "matrix=$(python scripts/ci/board_matrix.py --event '${{ github.event_name }}')" >> "$GITHUB_OUTPUT"

  board:
    needs: setup
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix: ${{ fromJSON(needs.setup.outputs.matrix) }}
    name: ${{ matrix.id }}
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: ${{ matrix.submodules }}

      - name: Install Rust
        uses: dtolnay/rust-toolchain@1.95.0

      - name: Cache dependencies
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: core-board-${{ matrix.id }}
          workspaces: |
            . -> target

      - name: Install ARM bare-metal toolchain
        if: contains(matrix.toolchains, 'arm-none-eabi')
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-arm-none-eabi

      - name: Install RISC-V target
        if: contains(matrix.toolchains, 'riscv')
        run: rustup target add riscv32imac-unknown-none-elf

      - name: Fetch device packs
        if: matrix.packs != ''
        uses: ./.github/actions/fetch-pack
        with:
          pack: ${{ matrix.packs }}

      - name: Build firmware
        run: ${{ matrix.path }}/ci/build.sh

      - name: Run tests
        run: ${{ matrix.path }}/ci/test.sh

      - name: Upload board artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: board-${{ matrix.id }}
          path: out/boards/${{ matrix.id }}/
          if-no-files-found: ignore
```

Note: `packs` is a single space-joined string; for now each gated board declares at
most one pack, so passing `matrix.packs` directly is correct. Multi-pack fan-out is a
later-phase concern (documented in the spec), not Phase 1.

- [ ] **Step 2: Lint the workflow YAML**

Run: `python -c "import yaml; yaml.safe_load(open('.github/workflows/core-board-ci.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/core-board-ci.yml
git commit -m "ci: add manifest-driven Core Board CI workflow"
```

---

## Task 5: Cut over and remove the old YAML

**Files:**
- Delete: `.github/workflows/core-iolink-station.yml`

- [ ] **Step 1: Open the PR (keep old + new running side by side first)**

```bash
git push -u origin refactor/board-ci-matrix
gh pr create --base main --title "Manifest-driven Core Board CI (Phase 1)" \
  --body "Replaces hand-written core-iolink-station.yml with a manifest-driven board-CI matrix. Phase 1 of the board-CI redesign (see docs/superpowers/specs/2026-06-28-board-ci-matrix-design.md). Migrates the iolink-station-l476 gate; check name preserved."
```

- [ ] **Step 2: Verify the new check is green with the preserved name**

Run: `gh pr checks --watch`
Expected: a check named `iolink-station-l476` from **Core Board CI** reports `pass`.
Do NOT proceed to Step 3 until this is green.

- [ ] **Step 3: Delete the superseded workflow**

```bash
git rm .github/workflows/core-iolink-station.yml
git commit -m "ci: remove core-iolink-station.yml, superseded by Core Board CI"
git push
```

- [ ] **Step 4: Re-verify green after removal**

Run: `gh pr checks --watch`
Expected: `iolink-station-l476` still `pass` (now sourced only from Core Board CI). No duplicate/orphaned check.

- [ ] **Step 5: Confirm branch-protection required-check still matches**

Verify in repo settings (or with the user) that the required check `iolink-station-l476` resolves to the new workflow's job. If branch protection pins by workflow name, update it to accept `Core Board CI / iolink-station-l476`.

---

## Self-Review

- **Spec coverage (Phase 1 scope):** manifest (Task 1 ✓), co-located execution (Task 3 ✓), single workflow setup+board jobs (Task 4 ✓), fetch-pack action (Task 2 ✓), public-proof check-name preservation (Task 4 `name:` + Task 5 verify ✓), delete old YAML after green (Task 5 ✓). Phases 2–5 deferred to their own plans by design.
- **No-skip-to-green constraint:** preserved via `LABWIRED_REQUIRE_IOLINK_ELFS=1` + `test -f` guards in Task 3, and manifest validation failing the `setup` job in Task 1.
- **Type/name consistency:** `load_manifest`/`validate`/`select`/`to_matrix` names match across Task 1 code, tests, and the workflow CLI call; `STM32CUBE_L4_DIR` produced by Task 2 and consumed by Task 3; `matrix.id`/`matrix.path`/`matrix.packs`/`matrix.submodules` keys produced by `to_matrix` and consumed by Task 4.
- **Placeholder scan:** none — all code blocks complete.
```
