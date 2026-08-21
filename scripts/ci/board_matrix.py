#!/usr/bin/env python3
"""Validate the board CI manifest and emit a GitHub Actions matrix."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import yaml

GATE_EVENTS = {"pull_request", "push"}


def load_manifest(path: str) -> list[dict]:
    data = yaml.safe_load(Path(path).read_text()) or {}
    return list(data.get("boards", []))


def validate(entries: list[dict], repo_root: str) -> list[str]:
    root = Path(repo_root)
    errors: list[str] = []
    seen: set[str] = set()
    if not entries:
        errors.append("manifest has no boards")
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
        if kind == "firmware-gate":
            for script in ("ci/build.sh", "ci/test.sh"):
                if not (path / script).is_file():
                    errors.append(f"{eid}: missing {e['path']}/{script}")
    return errors


def select(entries: list[dict], event: str) -> list[dict]:
    if event in GATE_EVENTS:
        return [e for e in entries if e.get("gate")]
    return list(entries)


def check_selection(selected: list[dict], entries: list[dict], event: str) -> list[str]:
    """Refuse to emit an EMPTY matrix for an event that is supposed to gate.

    `validate()` above answers "is this manifest well formed"; it cannot
    answer the question the workflow actually depends on. `core-board-ci.yml`
    pipes this script's stdout straight into `strategy.matrix` and has no
    aggregator job, so `{"include": []}` skips the `board` job and the whole
    workflow reports SUCCESS having compiled no firmware. A manifest with
    every entry `gate: false` is perfectly valid and produces exactly that.

    Today only the data keeps the gate alive: 2 of 5 entries in
    `configs/ci/boards.yml` carry `gate: true`. Flip those two flags and the
    board gate goes silent with a green tick — no error, no skipped-job
    warning, nothing to read in a log. So the emptiness of the SELECTED list
    is the assertion, not the emptiness of the manifest.

    Scoped to GATE_EVENTS on purpose: `schedule` and `workflow_dispatch` take
    every entry, so an empty selection there means an empty manifest, which
    `validate()` already rejects.
    """
    if event in GATE_EVENTS and not selected:
        return [
            f"event {event!r} selected 0 of {len(entries)} board(s): every entry "
            "is `gate: false`. That emits {\"include\": []}, which skips the "
            "`board` job in core-board-ci.yml and reports the workflow GREEN "
            "having built nothing. Mark at least one entry `gate: true`."
        ]
    return []


def to_matrix(entries: list[dict]) -> dict:
    include = []
    for e in entries:
        include.append({
            "id": e["id"],
            "kind": e["kind"],
            "path": e["path"],
            "apt": " ".join(e.get("apt", [])),
            "rust_targets": " ".join(e.get("rust_targets", [])),
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

    manifest = args.manifest if Path(args.manifest).is_absolute() else str(Path(args.repo_root) / args.manifest)
    entries = load_manifest(manifest)
    selected = select(entries, args.event)
    errors = validate(entries, args.repo_root) + check_selection(
        selected, entries, args.event
    )
    if errors:
        print("Manifest validation failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1
    print(json.dumps(to_matrix(selected)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
