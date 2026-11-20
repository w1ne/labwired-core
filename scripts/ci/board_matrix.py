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
        if kind in {"firmware-gate", "sim-validate"}:
            for script in ("ci/build.sh", "ci/test.sh"):
                if not (path / script).is_file():
                    errors.append(f"{eid}: missing {e['path']}/{script}")
    return errors


def select_push(entries: list[dict], changed_paths: list[str]) -> list[dict]:
    """Select gated examples affected by a push, failing closed on shared paths.

    Device catalog and coverage-scoreboard edits do not change a board example.
    A change inside one known example selects that example. Shared configs,
    shared example files, workflow helpers, and unknown paths keep the full gate.
    """
    gated = [e for e in entries if e.get("gate")]
    selected: dict[str, dict] = {}
    select_all = not changed_paths

    for changed in changed_paths:
        if changed.startswith("examples/"):
            matches = [
                e for e in entries
                if changed == e["path"] or changed.startswith(e["path"].rstrip("/") + "/")
            ]
            if matches:
                for entry in matches:
                    if entry.get("gate"):
                        selected[entry["id"]] = entry
            else:
                # Common examples and new/renamed manifest paths can affect
                # more than one target; retain the whole gate until mapped.
                select_all = True
        elif changed.startswith("configs/devices/"):
            continue
        elif changed in {
            "docs/coverage/tier1-matrix.json",
            "scripts/generate_tier1_scoreboard.py",
        }:
            continue
        else:
            # Includes chip/system configs, the board manifest, HIL/PlatformIO,
            # CI workflow/action changes, and any future relevant path.
            select_all = True

    return gated if select_all else list(selected.values())


def select(
    entries: list[dict], event: str, changed_paths: list[str] | None = None
) -> list[dict]:
    if event in GATE_EVENTS:
        if event == "push" and changed_paths is not None:
            return select_push(entries, changed_paths)
        return [e for e in entries if e.get("gate")]
    return list(entries)


def check_selection(
    selected: list[dict],
    entries: list[dict],
    event: str,
    changed_paths: list[str] | None = None,
) -> list[str]:
    """Refuse an empty matrix when the event is expected to select board jobs.

    `validate()` above answers "is this manifest well formed"; it cannot
    answer the question the workflow actually depends on. `core-board-ci.yml`
    pipes this script's stdout straight into `strategy.matrix` and has no
    aggregator job, so `{"include": []}` skips the `board` job and the whole
    workflow reports SUCCESS having compiled no firmware. A manifest with
    every entry `gate: false` is perfectly valid and produces exactly that.

    An unscoped gate run must select at least one board; otherwise a manifest
    with every entry `gate: false` silently skips the board job. A push with a
    changed-path list may intentionally select none when it only changes an
    unrelated device catalog, coverage file, or ungated example.

    `schedule` and `workflow_dispatch` take every entry, so an empty selection
    there means an empty manifest, which `validate()` already rejects.
    """
    if (
        event in GATE_EVENTS
        and not selected
        and not (event == "push" and changed_paths is not None)
    ):
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
    ap.add_argument("--changed-paths-file")
    args = ap.parse_args()

    manifest = args.manifest if Path(args.manifest).is_absolute() else str(Path(args.repo_root) / args.manifest)
    entries = load_manifest(manifest)
    changed_paths = None
    if args.changed_paths_file:
        changed_paths = Path(args.changed_paths_file).read_text().splitlines()
    selected = select(entries, args.event, changed_paths)
    errors = validate(entries, args.repo_root) + check_selection(
        selected, entries, args.event, changed_paths
    )
    if errors:
        print("Manifest validation failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1
    if args.event == "push" and changed_paths is not None and not selected:
        print("No gated board examples were affected by this push.", file=sys.stderr)
    print(json.dumps(to_matrix(selected)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
