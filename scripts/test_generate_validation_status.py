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

import yaml
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


# Inside the fixture ack's window (2026-06-01 + ACK_TTL_DAYS). The cases below
# assert the CONTENT rule, so they pin the clock rather than drift into the
# expiry rule as the wall clock moves past the fixture's ack. Expiry has its own
# cases further down.
IN_WINDOW = datetime.date(2026, 6, 15)


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

    failing = gvs.evaluate(board, today=IN_WINDOW)["failing"]
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
    assert gvs.evaluate(board, today=IN_WINDOW)["failing"] is False, "ack (06-01) >= newest (05-01) still covers"

    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 7, 1))
    assert gvs.evaluate(board, today=IN_WINDOW)["failing"] is True, "model newer than the ack still fails"


def test_write_ack_digests_never_acks_an_unacked_board(tmp_path, monkeypatch):
    """Stamping records WHICH tree was acked; it must never BE the ack."""
    board, _ = _board_with_digest(tmp_path, monkeypatch, b"fn model() {}\n")
    del board["drift_ack"]

    manifest_path = tmp_path / "manifest.yaml"
    manifest_path.write_text("boards:\n  - id: demo\n    models:\n      - models/periph.rs\n")
    monkeypatch.setattr(gvs, "MANIFEST", manifest_path)

    gvs.write_ack_digests({"boards": [board]})
    assert "drift_ack_digest" not in manifest_path.read_text()


def test_digests_reports_the_same_verdict_as_the_gate(tmp_path, monkeypatch, capsys):
    """`--digests` exists because the manifest documents it, and it must not be
    a second opinion: the manifest header tells authors to print the digest so
    they can write an ack pre-merge, which only helps if what it prints is what
    `--drift` will judge them on.
    """
    body = b"fn model() {}\n"
    board, model = _board_with_digest(tmp_path, monkeypatch, body)
    board["drift_ack_digest"] = gvs.model_digest(board["models"])
    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 5, 1))

    # Covered by the ack: the digest prints, and the gate agrees it is not failing.
    assert gvs.print_digests({"boards": [board]}) == 0
    printed = capsys.readouterr().out
    assert gvs.model_digest(board["models"]) in printed
    assert gvs.evaluate(board, today=IN_WINDOW)["status"] in printed
    assert not gvs.evaluate(board, today=IN_WINDOW)["failing"]

    # The negative control. Move a byte of the model: the printed digest must
    # change, the recorded ack must be shown as the thing that no longer
    # matches, and the gate must now be failing.
    model.write_bytes(body + b"// a real change\n")
    assert gvs.print_digests({"boards": [board]}) == 0
    printed = capsys.readouterr().out
    assert gvs.model_digest(board["models"]) in printed
    assert f"ack recorded {board['drift_ack_digest']}" in printed
    assert gvs.evaluate(board, today=IN_WINDOW)["failing"]


def test_digests_never_writes(tmp_path, monkeypatch, capsys):
    """A report, not a stamper. `--write-ack-digests` is the writing one."""
    board, _ = _board_with_digest(tmp_path, monkeypatch, b"fn model() {}\n")
    manifest_path = tmp_path / "manifest.yaml"
    original = "boards:\n  - id: demo\n    models:\n      - models/periph.rs\n"
    manifest_path.write_text(original)
    monkeypatch.setattr(gvs, "MANIFEST", manifest_path)
    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 5, 1))

    gvs.print_digests({"boards": [board]})
    capsys.readouterr()
    assert manifest_path.read_text() == original


# ── An ack lapses ────────────────────────────────────────────────────────────
#
# The digest rule made acks precise and, in doing so, PERMANENT: while the
# models hold still, an ack from any date keeps a drifted board green forever.
# Every acked board on the manifest reads "re-capture pending" and nothing ever
# made the pending part come due. An ack is a promise to re-capture; a promise
# with no date is a waiver.
#
# Two properties, and the second matters as much as the first: the GATE must
# move with the calendar, and the DOCUMENT must not.


def _acked_drifted_board(tmp_path, monkeypatch):
    """A board that has genuinely drifted and carries a content-covering ack."""
    board, model = _board_with_digest(tmp_path, monkeypatch, b"fn model() {}\n")
    board["drift_ack_digest"] = gvs.model_digest(board["models"])
    # Newer than the 2026-01-01 capture, so `drifted` is true and the ack is
    # the only thing keeping this board green.
    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 5, 1))
    return board


def test_a_fresh_ack_covers_the_board(tmp_path, monkeypatch):
    board = _acked_drifted_board(tmp_path, monkeypatch)
    v = gvs.evaluate(board, today=datetime.date(2026, 6, 2))
    assert v["drifted"] is True, "the fixture must actually be drifted or this proves nothing"
    assert v["expired"] is False
    assert v["failing"] is False


def test_the_ack_lapses_on_the_day_after_its_expiry(tmp_path, monkeypatch):
    """The whole point: unchanged content, unchanged manifest, red anyway."""
    board = _acked_drifted_board(tmp_path, monkeypatch)
    expires = board["drift_ack"] + datetime.timedelta(days=gvs.ACK_TTL_DAYS)

    assert gvs.evaluate(board, today=expires)["failing"] is False, "valid through its last day"
    lapsed = gvs.evaluate(board, today=expires + datetime.timedelta(days=1))
    assert lapsed["expired"] is True
    assert lapsed["failing"] is True, "an expired ack must stop covering the board"


def test_an_explicit_expiry_overrides_the_default_window(tmp_path, monkeypatch):
    """For a re-capture that is genuinely blocked — reviewable, unlike forever."""
    board = _acked_drifted_board(tmp_path, monkeypatch)
    board["drift_ack_expires"] = datetime.date(2027, 1, 1)

    assert gvs.evaluate(board, today=datetime.date(2026, 12, 31))["failing"] is False
    assert gvs.evaluate(board, today=datetime.date(2027, 1, 2))["failing"] is True


def test_expiry_cannot_fail_a_board_that_never_drifted(tmp_path, monkeypatch):
    """A stale ack on a board whose models never moved is not a finding."""
    board = _acked_drifted_board(tmp_path, monkeypatch)
    # Model older than the capture: nothing drifted, so nothing to acknowledge.
    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2025, 6, 1))
    v = gvs.evaluate(board, today=datetime.date(2099, 1, 1))
    assert v["drifted"] is False
    assert v["failing"] is False


def test_a_board_with_no_ack_is_unchanged_by_expiry(tmp_path, monkeypatch):
    board = _acked_drifted_board(tmp_path, monkeypatch)
    del board["drift_ack"]
    del board["drift_ack_digest"]
    v = gvs.evaluate(board, today=datetime.date(2026, 6, 2))
    assert v["expires"] is None
    assert v["expired"] is False
    assert v["failing"] is True, "still failing for the original reason — unacked drift"


def test_the_generated_document_does_not_move_with_the_calendar(tmp_path, monkeypatch):
    """The other half of the contract, and the easy half to get wrong.

    `--check` diffs a COMMITTED document. If an expiry verdict leaked into a row,
    the doc would go stale on a day nobody committed and every later PR would
    demand a regen commit carrying no information — exactly the #834 / #798
    defect the digest rule was introduced to end. The row states the two dates
    (both from the manifest, both still); only the gate reads the clock.
    """
    board = _acked_drifted_board(tmp_path, monkeypatch)
    board.update({"tier": "silicon-verified", "doc": "boards/demo.md", "chip": "demo"})
    manifest = {"boards": [board]}

    baseline = gvs.render(manifest, today=datetime.date(2026, 6, 2))
    for far in (datetime.date(2026, 7, 5), datetime.date(2027, 1, 1), datetime.date(2099, 1, 1)):
        assert gvs.render(manifest, today=far) == baseline, (
            f"the document changed at {far} — an expiry verdict leaked into a row, "
            "which makes the committed doc rot on a day nobody committed"
        )

    # And the row must still SHOW the expiry, or a reader cannot see it coming.
    assert "expires 2026-07-01" in baseline


def test_the_committed_manifest_is_not_already_expired():
    """Reads the real manifest: this change must cost nothing on the day it lands.

    Not a restatement of the manifest — it asserts the property that made this
    safe to merge, and it will fail loudly the day a real ack comes due, which
    is the entire intent.
    """
    manifest = yaml.safe_load(gvs.MANIFEST.read_text())
    stale = [
        b["id"]
        for b in manifest["boards"]
        if gvs.evaluate(b, today=datetime.date.today())["expired"]
    ]
    assert not stale, (
        "drift acks have come due: " + ", ".join(stale) + ". Re-capture and bump "
        "silicon.last_capture, or renew the ack — do not widen ACK_TTL_DAYS to clear this."
    )

def test_write_ack_digests_leaves_undrifted_boards_alone(tmp_path, monkeypatch):
    """The stamper must not sweep boards whose digest nothing reads.

    A `drift_ack_digest` is consulted by `evaluate()` for exactly one purpose:
    deciding whether an ack still covers a DRIFTED board. Stamping a board that
    is not drifted rewrites a line no gate consults, and on 2026-08-31 that put
    the SAME four unrelated boards into four separate pull requests, each time
    to be reverted by hand.
    """
    boards = [
        {"id": "drifted-one", "drift_ack": "2026-08-31", "drift_ack_digest": "stale",
         "models": ["m.rs"], "silicon": {"last_capture": "2020-01-01"}},
        {"id": "fresh-one", "drift_ack": "2026-08-31", "drift_ack_digest": "stale",
         "models": ["m.rs"], "silicon": {"last_capture": "2099-01-01"}},
    ]
    seen = {"drifted": False, "fresh": False}

    def fake_evaluate(board, today=None):
        return {"drifted": board["id"] == "drifted-one"}

    def fake_digest(models):
        return "newdigest"

    monkeypatch.setattr(gvs, "evaluate", fake_evaluate)
    monkeypatch.setattr(gvs, "model_digest", fake_digest)

    manifest_text = (
        "boards:\n"
        "  - id: drifted-one\n"
        "    drift_ack: 2026-08-31\n"
        "    drift_ack_digest: stale\n"
        "  - id: fresh-one\n"
        "    drift_ack: 2026-08-31\n"
        "    drift_ack_digest: stale\n"
    )
    manifest_file = tmp_path / "manifest.yaml"
    manifest_file.write_text(manifest_text)
    monkeypatch.setattr(gvs, "MANIFEST", manifest_file)

    gvs.write_ack_digests({"boards": boards})
    after = manifest_file.read_text()

    drifted_line = [ln for ln in after.splitlines() if "drift_ack_digest" in ln][0]
    fresh_line = [ln for ln in after.splitlines() if "drift_ack_digest" in ln][1]
    assert "newdigest" in drifted_line, "a drifted board must be stamped"
    assert "stale" in fresh_line, "an undrifted board must be left alone"

    # And the escape hatch still does the old thing, deliberately.
    manifest_file.write_text(manifest_text)
    gvs.write_ack_digests({"boards": boards}, stamp_all=True)
    lines = [ln for ln in manifest_file.read_text().splitlines() if "drift_ack_digest" in ln]
    assert all("newdigest" in ln for ln in lines), "--all must stamp both"
