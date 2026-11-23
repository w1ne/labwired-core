#!/usr/bin/env python3
"""Validate the board CI manifest and emit a GitHub Actions matrix."""
from __future__ import annotations

import argparse
import json
import posixpath
import re
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


# Mirror of `on.push.paths` in core-board-ci.yml (a test keeps them equal). The
# push diff the workflow feeds in is the whole commit range, so unrelated paths
# (crates/, docs/, validation/ ...) that ride along with a relevant change must
# not fall through to "unknown path, run every board".
TRIGGER_PATHS = (
    "configs/**",
    "examples/**",
    "hil/**",
    "platformio/**",
    ".github/actions/apt-install-cached/**",
    ".github/actions/fetch-pack/**",
    "scripts/ci/board_matrix.py",
    "configs/ci/boards.yml",
    "scripts/generate_tier1_scoreboard.py",
    "docs/coverage/tier1-matrix.json",
    ".github/workflows/core-board-ci.yml",
)


def _triggers(path: str) -> bool:
    for pat in TRIGGER_PATHS:
        if pat.endswith("/**"):
            if path.startswith(pat[:-2]):
                return True
        elif path == pat:
            return True
    return False


_TOKEN = re.compile(r"[A-Za-z0-9_.\-/]+")
_TEXT_EXT = {
    ".yaml", ".yml", ".sh", ".toml", ".json", ".py", ".txt", ".cfg", ".ini", ".mk",
    ".c", ".h", ".cpp", ".ino", ".ld", ".cmake", ".rs",
}
_TEXT_NAMES = {"Makefile"}
_SKIP_DIRS = {"evidence", "logs", "out", "target", "node_modules", ".git"}
_MAX_BYTES = 256 * 1024


def _tokens_of_file(path: Path) -> set[str]:
    if path not in _FILE_TOKENS:
        try:
            ok = path.stat().st_size <= _MAX_BYTES
            _FILE_TOKENS[path] = set(_TOKEN.findall(path.read_text(errors="ignore"))) if ok else set()
        except OSError:
            _FILE_TOKENS[path] = set()
    return _FILE_TOKENS[path]


_FILE_TOKENS: dict[Path, set[str]] = {}
_CFG_FILES: dict[str, tuple[set[str], dict[str, list[str]]]] = {}


def _config_files(repo_root: str) -> tuple[set[str], dict[str, list[str]]]:
    """All config files (repo-relative) and bare chip/system stems -> files."""
    if repo_root not in _CFG_FILES:
        root = Path(repo_root)
        files: set[str] = set()
        stems: dict[str, list[str]] = {}
        for p in (root / "configs").rglob("*"):
            if p.is_file() and p.suffix in _TEXT_EXT:
                rel = p.relative_to(root).as_posix()
                files.add(rel)
                if p.parent.name in {"chips", "systems"}:
                    stems.setdefault(p.stem, []).append(rel)
        _CFG_FILES[repo_root] = (files, stems)
    return _CFG_FILES[repo_root]


def _resolve(root: Path, src_dir: str, tok: str, files: set[str], stems: dict[str, list[str]]) -> list[str]:
    """Config files a token can refer to: path relative to the source file or
    repo root, or a bare chip/system name (`chip: esp32c6`)."""
    out = []
    for base in (src_dir, ""):
        cand = posixpath.normpath(posixpath.join(base, tok))
        if cand in files:
            out.append(cand)
    if "/" not in tok and tok in stems:
        out.extend(stems[tok])
    return out


def board_inputs(entry: dict, repo_root: str) -> set[str]:
    """Config files a board can read, as repo-relative paths.

    Seeds with every token in the board's own scripts and descriptors
    (evidence/log dirs excluded), resolves each as a path (relative to the
    file or the repo root) or a bare chip/system name, then follows the
    resolved configs transitively. A mention in a comment counts, so this
    over-selects rather than skips.
    """
    root = Path(repo_root)
    files, stems = _config_files(repo_root)
    board_dir = root / entry["path"]
    work: list[tuple[str, set[str]]] = []
    for p in board_dir.rglob("*"):
        if not p.is_file() or not (p.suffix in _TEXT_EXT or p.name in _TEXT_NAMES):
            continue
        if _SKIP_DIRS & set(p.relative_to(board_dir).parts[:-1]):
            continue
        work.append((p.parent.relative_to(root).as_posix(), _tokens_of_file(p)))
    reached: set[str] = set()
    while work:
        src_dir, toks = work.pop()
        for t in toks:
            for rel in _resolve(root, src_dir, t, files, stems):
                if rel not in reached:
                    reached.add(rel)
                    work.append((posixpath.dirname(rel), _tokens_of_file(root / rel)))
    return reached


def board_refers_to(entry: dict, repo_root: str, changed: str) -> bool:
    """True if any non-evidence file of the board names `changed`, or a
    directory containing it (deeper than `examples/`), as a path."""
    root = Path(repo_root)
    board_dir = root / entry["path"]
    for p in board_dir.rglob("*"):
        if not p.is_file() or not (p.suffix in _TEXT_EXT or p.name in _TEXT_NAMES):
            continue
        if _SKIP_DIRS & set(p.relative_to(board_dir).parts[:-1]):
            continue
        src_dir = p.parent.relative_to(root).as_posix()
        for tok in _tokens_of_file(p):
            for base in (src_dir, ""):
                cand = posixpath.normpath(posixpath.join(base, tok))
                if cand == changed or (
                    cand.count("/") >= 1 and changed.startswith(cand + "/")
                ):
                    return True
    return False


def select_push(
    entries: list[dict], changed_paths: list[str], repo_root: str | None = None
) -> list[dict]:
    """Select gated examples affected by a push, failing closed on shared paths.

    Device catalog and coverage-scoreboard edits do not change a board example.
    A change inside one known example selects that example. Shared configs,
    shared example files, workflow helpers, and unknown paths keep the full gate.
    """
    gated = [e for e in entries if e.get("gate")]
    selected: dict[str, dict] = {}
    select_all = not changed_paths
    inputs_cache: dict[str, set[str]] = {}

    def owners(changed: str) -> list[dict]:
        """Gated boards whose input closure contains `changed`."""
        if repo_root is None:
            return []
        hit = []
        for e in gated:
            if e["id"] not in inputs_cache:
                inputs_cache[e["id"]] = board_inputs(e, repo_root)
            if changed in inputs_cache[e["id"]]:
                hit.append(e)
        return hit

    for changed in changed_paths:
        if not _triggers(changed):
            continue
        if changed.startswith("examples/"):
            matches = [
                e for e in entries
                if changed == e["path"] or changed.startswith(e["path"].rstrip("/") + "/")
            ]
            if matches:
                for entry in matches:
                    if entry.get("gate"):
                        selected[entry["id"]] = entry
            elif repo_root is not None and not any(
                board_refers_to(e, repo_root, changed)
                for e in gated
            ):
                # An example directory outside the manifest that no gated
                # board's scripts or descriptors point at.
                continue
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
        elif changed.startswith("configs/") and (hit := owners(changed)):
            # Chip/system/peripheral descriptor read only by these boards.
            for entry in hit:
                selected[entry["id"]] = entry
        elif repo_root is not None and changed.startswith(
            ("configs/chips/", "configs/systems/", "configs/peripherals/", "configs/onboarding/")
        ):
            # Descriptor no gated board's closure reaches: no gated job reads it.
            continue
        else:
            # Includes chip/system configs, the board manifest, HIL/PlatformIO,
            # CI workflow/action changes, and any future relevant path.
            select_all = True

    return gated if select_all else list(selected.values())


def select(
    entries: list[dict],
    event: str,
    changed_paths: list[str] | None = None,
    repo_root: str | None = None,
) -> list[dict]:
    if event in GATE_EVENTS:
        if event == "push" and changed_paths is not None:
            return select_push(entries, changed_paths, repo_root)
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
    pipes this script's stdout into `strategy.matrix`, gates the `board` job
    on `has_boards` (GitHub Actions fails a run whose matrix has an empty
    `include`), and has no aggregator job. So `{"include": []}` skips the
    `board` job and the whole workflow reports SUCCESS having compiled no
    firmware. A manifest with every entry `gate: false` is perfectly valid
    and produces exactly that.

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
    selected = select(entries, args.event, changed_paths, args.repo_root)
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
