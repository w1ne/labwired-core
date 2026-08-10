# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Tests for the incomplete-history guard in generate_validation_status.py.

The rendering itself is exercised by CI running the generator with
`--check --drift` against the committed doc. What is tested here is the property
that actually failed in practice, and that no gate could observe: on a truncated
history the generator did not fail, it produced a plausible document with wrong
dates. `ci-fixture-riscv` rendered the graft commit's 2026-08-04 where the truth
on full history is 2026-03-09, and seven other boards were skewed the same way.

The shallow case is tested against a REAL shallow clone rather than a stubbed
`git rev-parse`, because the thing worth pinning down is git's behaviour, not our
belief about it — a stub would have happily agreed with the broken version too.
"""

import datetime
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import generate_validation_status as gvs  # noqa: E402


def git(cwd, *args):
    subprocess.run(["git", *args], cwd=str(cwd), check=True, capture_output=True, text=True)


def make_repo(root: Path, commits: int = 3) -> Path:
    """A throwaway repo with `commits` commits touching the same file."""
    root.mkdir(parents=True, exist_ok=True)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.email", "t@example.invalid")
    git(root, "config", "user.name", "test")
    for i in range(commits):
        (root / "model.txt").write_text(f"rev {i}\n")
        git(root, "add", "model.txt")
        git(root, "commit", "-q", "-m", f"rev {i}")
    return root


# ── The real checkout must stay clean, or this guard is a build-breaker ───────


def test_this_checkout_has_no_defect():
    assert gvs.history_defect() is None


# ── Truncation that git itself reports ───────────────────────────────────────


def test_shallow_clone_is_rejected(tmp_path, monkeypatch):
    src = make_repo(tmp_path / "src")
    dst = tmp_path / "shallow"
    # file:// and not a plain path: git silently ignores --depth for local clones.
    git(tmp_path, "clone", "-q", "--depth", "1", f"file://{src}", str(dst))
    monkeypatch.setattr(gvs, "CORE_ROOT", dst)
    defect = gvs.history_defect()
    assert defect is not None and "shallow" in defect


def test_unshallowed_clone_is_accepted(tmp_path, monkeypatch):
    """The remedy the error message prints must actually clear the guard."""
    src = make_repo(tmp_path / "src")
    dst = tmp_path / "shallow"
    git(tmp_path, "clone", "-q", "--depth", "1", f"file://{src}", str(dst))
    git(dst, "fetch", "-q", "--unshallow")
    monkeypatch.setattr(gvs, "CORE_ROOT", dst)
    assert gvs.history_defect() is None


def test_not_a_repository_is_rejected(tmp_path, monkeypatch):
    # Degrades to every date rendering "—" and a drift gate that finds nothing.
    monkeypatch.setattr(gvs, "CORE_ROOT", tmp_path)
    defect = gvs.history_defect()
    assert defect is not None and "not a git checkout" in defect


# ── Truncation git does NOT report: a rewritten parent chain ─────────────────


def test_replace_ref_is_rejected(tmp_path, monkeypatch):
    root = make_repo(tmp_path / "grafted")
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=str(root), capture_output=True, text=True, check=True
    ).stdout.strip()
    git(root, "replace", "--graft", head)  # re-parent HEAD as a root commit
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    defect = gvs.history_defect()
    assert defect is not None and "replace refs" in defect


def test_grafts_file_is_rejected(tmp_path, monkeypatch):
    root = make_repo(tmp_path / "legacy-graft")
    (root / ".git" / "info").mkdir(parents=True, exist_ok=True)
    (root / ".git" / "info" / "grafts").write_text("")
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    defect = gvs.history_defect()
    assert defect is not None and "grafted history" in defect


# ── Partial clones: blob-only is exact, tree-omitting is not ─────────────────


@pytest.mark.parametrize("spec", ["blob:none", "blob:limit=1k"])
def test_blob_only_partial_clone_is_allowed(tmp_path, monkeypatch, spec):
    """Commit and tree objects are all local, so the dates are exact."""
    root = make_repo(tmp_path / "partial")
    git(root, "config", "remote.origin.partialclonefilter", spec)
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    assert gvs.history_defect() is None


def test_tree_omitting_partial_clone_is_rejected(tmp_path, monkeypatch):
    root = make_repo(tmp_path / "partial-tree")
    git(root, "config", "remote.origin.partialclonefilter", "tree:0")
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    defect = gvs.history_defect()
    assert defect is not None and "tree-omitting" in defect


# ── The guard itself: hard stop, actionable remedy ───────────────────────────


def test_require_full_history_exits_with_the_fix(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(gvs, "CORE_ROOT", tmp_path)
    with pytest.raises(SystemExit) as e:
        gvs.require_full_history()
    # 2 = precondition failure (as for missing PyYAML), not 1 = gate verdict;
    # a caller must be able to tell "I cannot judge" from "the boards drifted".
    assert e.value.code == 2
    err = capsys.readouterr().err
    assert "git fetch --unshallow" in err
    assert "fetch-depth: 0" in err


# ── Drift acks are pinned to CONTENT, not to a timestamp (#834) ──────────────
#
# The date rule got both directions wrong. A squash merge stamps the merge time
# onto every file a PR touched, so an ack written on the branch stopped covering
# the very tree it was reviewed against and main went red on unchanged content.
# In the other direction, `ack >= newest` accepted ANY content as long as the
# ack was dated on or after the newest model commit, so an edit landing the same
# day an ack was written slipped through.
#
# With a `drift_ack_digest` recorded, the verdict depends only on content. These
# four cases are the whole contract.


def _board_with_digest(tmp_path, monkeypatch, model_body: bytes):
    """A one-board manifest whose ack is pinned to `model_body`'s digest."""
    root = tmp_path / "repo"
    (root / "models").mkdir(parents=True)
    model = root / "models" / "periph.rs"
    model.write_bytes(model_body)
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    return {
        "id": "demo",
        "silicon": {"last_capture": datetime.date(2026, 1, 1)},
        "drift_ack": datetime.date(2026, 6, 1),
        "models": ["models/periph.rs"],
    }, model


@pytest.mark.parametrize("redated", [False, True], ids=["dates-unchanged", "squash-redated"])
@pytest.mark.parametrize("mutated", [False, True], ids=["content-same", "content-CHANGED"])
def test_digest_ack_tracks_content_not_dates(tmp_path, monkeypatch, redated, mutated):
    body = b"fn model() {}\n"
    board, model = _board_with_digest(tmp_path, monkeypatch, body)
    board["drift_ack_digest"] = gvs.model_digest(board["models"])

    if mutated:
        model.write_bytes(body + b"// a real change\n")
    # A squash merge re-dates the file far past every ack without touching it.
    monkeypatch.setattr(
        gvs,
        "newest_commit_date",
        lambda paths: datetime.date(2099, 1, 1) if redated else datetime.date(2026, 5, 1),
    )

    failing = gvs.evaluate(board)["failing"]
    assert failing is mutated, (
        f"redated={redated} mutated={mutated}: a digest-pinned ack must fail iff the "
        "model CONTENT moved — a re-date alone must not fail, and a same-day edit "
        "must not pass"
    )


def test_ack_without_a_digest_keeps_the_legacy_date_rule(tmp_path, monkeypatch):
    """Un-stamped acks must keep working, or adding the field breaks every board."""
    board, _ = _board_with_digest(tmp_path, monkeypatch, b"fn model() {}\n")
    assert "drift_ack_digest" not in board

    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 5, 1))
    assert gvs.evaluate(board)["failing"] is False, "ack (06-01) >= newest (05-01) still covers"

    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 7, 1))
    assert gvs.evaluate(board)["failing"] is True, "model newer than the ack still fails"


def test_write_ack_digests_never_acks_an_unacked_board(tmp_path, monkeypatch):
    """Stamping records WHICH tree was acked; it must never BE the ack."""
    board, _ = _board_with_digest(tmp_path, monkeypatch, b"fn model() {}\n")
    del board["drift_ack"]

    manifest_path = tmp_path / "manifest.yaml"
    manifest_path.write_text("boards:\n  - id: demo\n    models:\n      - models/periph.rs\n")
    monkeypatch.setattr(gvs, "MANIFEST", manifest_path)

    gvs.write_ack_digests({"boards": [board]})
    assert "drift_ack_digest" not in manifest_path.read_text()
