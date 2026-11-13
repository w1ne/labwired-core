# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Regression tests for source/capture reporting and its integrity gates.

Real Git fixtures cover incomplete-history failures. Source-content fixtures
prove that optional human ACKs never block integration or upgrade physical
capture evidence, while invalid evidence, watch holes and a committed report still fail.
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
    # Missing repository history is still invalid input for provenance reporting.
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
    # history integrity remains a precondition, independent of source drift.
    assert e.value.code == 2
    err = capsys.readouterr().err
    assert "git fetch --unshallow" in err
    assert "fetch-depth: 0" in err


# Source/capture status uses captured bytes, not source-review ACKs or dates.


def _board_with_capture(tmp_path, monkeypatch):
    root = tmp_path / "repo"
    (root / "models").mkdir(parents=True)
    model = root / "models" / "periph.rs"
    model.write_bytes(b"fn model() {}\n")
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2026, 1, 1))
    board = {
        "id": "demo",
        "tier": "silicon-verified",
        "doc": "boards/demo.md",
        "chip": "demo.yaml",
        "silicon": {"last_capture": datetime.date(2026, 1, 1)},
        "models": ["models/periph.rs"],
    }
    board["silicon"]["models_digest"] = gvs.model_digest(board["models"])
    return board, model


def _configure_cli(monkeypatch, board, flags):
    manifest = {"boards": [board]}
    path = gvs.CORE_ROOT / "manifest.yaml"
    path.write_text(yaml.safe_dump(manifest, sort_keys=False))
    output = gvs.CORE_ROOT / "VALIDATION_STATUS.md"
    output.write_text(gvs.render(manifest))
    monkeypatch.setattr(gvs, "MANIFEST", path)
    monkeypatch.setattr(gvs, "OUT_DOC", output)
    # These cases isolate policy. The tests above exercise the real history guard.
    monkeypatch.setattr(gvs, "require_full_history", lambda: None)
    monkeypatch.setattr(sys, "argv", ["generate_validation_status.py", *flags])
    return path, output


@pytest.mark.parametrize("redated", [False, True], ids=["same-date", "squash-redated"])
@pytest.mark.parametrize("mutated", [False, True], ids=["same-content", "changed-content"])
def test_capture_digest_tracks_content_not_dates(tmp_path, monkeypatch, redated, mutated):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    if mutated:
        model.write_bytes(model.read_bytes() + b"// real source change\n")
    if redated:
        monkeypatch.setattr(gvs, "newest_commit_date", lambda paths: datetime.date(2099, 1, 1))

    verdict = gvs.evaluate(board)
    assert verdict["source_match"] is not mutated
    assert verdict["drifted"] is mutated
    assert ("source changed" in verdict["status"]) is mutated


def test_uncommitted_source_edit_is_visible_without_a_new_git_date(tmp_path, monkeypatch):
    root = make_repo(tmp_path / "git-repo")
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    models = ["model.txt"]
    captured_date = gvs.newest_commit_date(models)
    board = {
        "id": "uncommitted",
        "models": models,
        "silicon": {"last_capture": captured_date, "models_digest": gvs.model_digest(models)},
    }
    assert gvs.evaluate(board)["source_match"] is True
    (root / "model.txt").write_text("uncommitted edit\n")
    assert gvs.newest_commit_date(models) == captured_date
    assert gvs.evaluate(board)["source_match"] is False


@pytest.mark.parametrize("ack", ["absent", "current", "expired"])
def test_missing_capture_digest_never_becomes_fresh(tmp_path, monkeypatch, ack):
    board, _ = _board_with_capture(tmp_path, monkeypatch)
    board["silicon"].pop("models_digest")
    if ack != "absent":
        board["drift_ack"] = "2026-01-01"
        board["drift_ack_digest"] = gvs.model_digest(board["models"])
        board["drift_ack_expires"] = "2020-01-01" if ack == "expired" else "2099-01-01"

    verdict = gvs.evaluate(board)
    assert verdict["source_match"] is None
    assert verdict["drifted"] is False, "unknown provenance is distinct from a known mismatch"
    assert verdict["status"] == "source match unverified — capture has no model digest"


@pytest.mark.parametrize("source", ["matching", "changed", "unverified", "no-capture"])
def test_ack_metadata_and_clock_do_not_change_capture_status(tmp_path, monkeypatch, source):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    if source == "changed":
        model.write_text("changed source\n")
    elif source == "unverified":
        board["silicon"].pop("models_digest")
    elif source == "no-capture":
        board.pop("silicon")
    baseline = gvs.evaluate(board)

    board.update({
        "drift_ack": "2026-01-01",
        "drift_ack_digest": gvs.model_digest(board["models"]),
        "drift_ack_expires": "2026-02-01",
    })
    for when in (datetime.date(2020, 1, 1), datetime.date(2026, 1, 2), datetime.date(2099, 1, 1)):
        assert gvs.evaluate(board, today=when) == baseline
    board["drift_ack_digest"] = "0" * 64
    assert gvs.evaluate(board) == baseline


@pytest.mark.parametrize("flags", [["--drift"], ["--check", "--drift"]])
@pytest.mark.parametrize("source", ["changed", "unverified"])
@pytest.mark.parametrize("ack", ["absent", "expired"])
def test_source_only_drift_is_informational(tmp_path, monkeypatch, capsys, flags, source, ack):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    if source == "changed":
        model.write_text("changed source\n")
    else:
        board["silicon"].pop("models_digest")
    if ack == "expired":
        board.update({
            "drift_ack": "2020-01-01",
            "drift_ack_digest": "0" * 64,
            "drift_ack_expires": "2020-02-01",
        })
    manifest_path, output = _configure_cli(monkeypatch, board, flags)
    before = (manifest_path.read_bytes(), output.read_bytes())

    assert gvs.main() == 0
    printed = capsys.readouterr()
    assert "informational; no source-review ACK required" in printed.out
    assert gvs.evaluate(board)["status"] in printed.out
    assert "ERROR" not in printed.err
    assert (manifest_path.read_bytes(), output.read_bytes()) == before


def test_check_does_not_depend_on_any_committed_report(tmp_path, monkeypatch):
    """The report is generated output: a stale or absent file is not a failure."""
    board, model = _board_with_capture(tmp_path, monkeypatch)
    model.write_text("changed source\n")
    _, output = _configure_cli(monkeypatch, board, ["--check", "--drift"])
    output.write_text("stale document\n")
    assert gvs.main() == 0
    assert output.read_text() == "stale document\n", "--check must not rewrite the file"
    output.unlink()
    assert gvs.main() == 0
    assert not output.exists()


def test_check_output_writes_the_rendered_report(tmp_path, monkeypatch):
    board, _ = _board_with_capture(tmp_path, monkeypatch)
    target = tmp_path / "published" / "VALIDATION_STATUS.md"
    _configure_cli(monkeypatch, board, ["--check", "--drift", "--output", str(target)])
    assert gvs.main() == 0
    assert target.read_text() == gvs.render({"boards": [board]})


def test_check_fails_when_the_generated_report_is_committed(tmp_path, monkeypatch, capsys):
    root = make_repo(tmp_path / "git-repo")
    monkeypatch.setattr(gvs, "CORE_ROOT", root)
    board = {"id": "demo", "tier": "structural", "doc": "d.md", "chip": "c.yaml", "models": ["model.txt"]}
    manifest = root / "manifest.yaml"
    manifest.write_text(yaml.safe_dump({"boards": [board]}, sort_keys=False))
    output = root / "docs" / "boards" / "VALIDATION_STATUS.md"
    output.parent.mkdir(parents=True)
    output.write_text("committed\n")
    monkeypatch.setattr(gvs, "MANIFEST", manifest)
    monkeypatch.setattr(gvs, "OUT_DOC", output)
    monkeypatch.setattr(sys, "argv", ["generate_validation_status.py", "--check"])
    assert gvs.main() == 0, "untracked generated report is fine"
    git(root, "add", "-f", "docs/boards/VALIDATION_STATUS.md")
    assert gvs.main() == 1
    assert "is tracked by git" in capsys.readouterr().err


def test_the_repository_does_not_track_the_generated_report():
    tracked = subprocess.run(
        ["git", "ls-files", "docs/boards/VALIDATION_STATUS.md"],
        cwd=str(gvs.CORE_ROOT), capture_output=True, text=True,
    ).stdout.strip()
    ignored = subprocess.run(
        ["git", "check-ignore", "-q", "docs/boards/VALIDATION_STATUS.md"], cwd=str(gvs.CORE_ROOT)
    ).returncode
    assert tracked == "" and ignored == 0


@pytest.mark.parametrize("defect", ["missing-model", "unwatched-peripheral"])
def test_watch_integrity_still_fails_with_informational_drift(tmp_path, monkeypatch, capsys, defect):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    board["silicon"].pop("models_digest")
    if defect == "missing-model":
        model.unlink()
    else:
        (gvs.CORE_ROOT / "demo.yaml").write_text('peripherals:\n  p:\n    path: "unwatched.yaml"\n')
        (gvs.CORE_ROOT / "unwatched.yaml").write_text("registers: []\n")
    _configure_cli(monkeypatch, board, ["--drift"])

    assert gvs.main() == 1
    error = capsys.readouterr().err
    assert ("do not exist" if defect == "missing-model" else "NOT covered") in error


@pytest.mark.parametrize("digest", [None, "", "not-a-digest", 123, "f" * 63])
def test_malformed_present_capture_digest_is_rejected(tmp_path, monkeypatch, digest):
    board, _ = _board_with_capture(tmp_path, monkeypatch)
    board["silicon"]["models_digest"] = digest
    with pytest.raises(ValueError, match="models_digest"):
        gvs.evaluate(board)
    with pytest.raises(SystemExit, match="models_digest"):
        gvs.load_manifest(yaml.safe_dump({"boards": [board]}))


@pytest.mark.parametrize("capture", [{}, {"last_capture": None}, {"last_capture": "not-a-date"}, "invalid"])
def test_missing_or_malformed_capture_record_is_rejected(tmp_path, monkeypatch, capture):
    board, _ = _board_with_capture(tmp_path, monkeypatch)
    board["silicon"] = capture
    with pytest.raises(SystemExit, match="silicon"):
        gvs.load_manifest(yaml.safe_dump({"boards": [board]}))


def test_invalid_capture_cannot_be_published_as_informational_drift(tmp_path, monkeypatch):
    board, _ = _board_with_capture(tmp_path, monkeypatch)
    path, output = _configure_cli(monkeypatch, board, ["--drift"])
    board["silicon"]["models_digest"] = "invalid"
    path.write_text(yaml.safe_dump({"boards": [board]}))
    before = output.read_bytes()
    with pytest.raises(SystemExit, match="models_digest"):
        gvs.main()
    assert output.read_bytes() == before


def test_the_generated_document_does_not_move_with_the_calendar(tmp_path, monkeypatch):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    model.write_text("changed source\n")
    board.update({"drift_ack": "2026-01-01", "drift_ack_expires": "2026-02-01"})
    manifest = {"boards": [board]}
    baseline = gvs.render(manifest, today=datetime.date(2026, 1, 2))
    for when in (datetime.date(2026, 2, 2), datetime.date(2099, 1, 1)):
        assert gvs.render(manifest, today=when) == baseline
    assert "source changed since capture" in baseline
    assert "expires" not in baseline


def test_committed_capture_records_do_not_gain_implicit_source_matches():
    manifest = gvs.load_manifest(gvs.MANIFEST.read_text())
    for board in manifest["boards"]:
        silicon = board.get("silicon")
        if silicon is not None and "models_digest" not in silicon:
            assert gvs.evaluate(board)["source_match"] is None


def test_digests_reports_capture_provenance_and_historical_ack(tmp_path, monkeypatch, capsys):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    board["drift_ack"] = "2020-01-01"
    board["drift_ack_digest"] = "0" * 64
    model.write_text("changed source\n")
    path, output = _configure_cli(monkeypatch, board, ["--digests"])
    before = (path.read_bytes(), output.read_bytes())

    assert gvs.main() == 0
    printed = capsys.readouterr().out
    assert gvs.model_digest(board["models"]) in printed
    assert gvs.evaluate(board)["status"] in printed
    assert f"capture recorded {board['silicon']['models_digest']}" in printed
    assert f"historical ack recorded {board['drift_ack_digest']}" in printed
    assert (path.read_bytes(), output.read_bytes()) == before


def test_default_generation_never_stamps_ack_or_capture_fields(tmp_path, monkeypatch):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    model.write_text("changed source\n")
    board["drift_ack"] = "2020-01-01"
    board["drift_ack_digest"] = "0" * 64
    path, _ = _configure_cli(monkeypatch, board, [])
    before = path.read_bytes()
    assert gvs.main() == 0
    assert path.read_bytes() == before


def test_write_ack_digests_never_creates_an_ack(tmp_path, monkeypatch):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    model.write_text("changed source\n")
    path, _ = _configure_cli(monkeypatch, board, [])
    before = path.read_bytes()
    assert gvs.write_ack_digests({"boards": [board]}) == 0
    assert path.read_bytes() == before


def test_legacy_ack_stamper_cannot_upgrade_a_capture(tmp_path, monkeypatch):
    board, model = _board_with_capture(tmp_path, monkeypatch)
    model.write_text("changed source\n")
    board["drift_ack"] = "2020-01-01"
    board["drift_ack_digest"] = "0" * 64
    path, _ = _configure_cli(monkeypatch, board, [])
    before = gvs.evaluate(board)
    assert gvs.write_ack_digests({"boards": [board]}) == 0
    after = gvs.load_manifest(path.read_text())["boards"][0]
    assert after["drift_ack_digest"] == gvs.model_digest(board["models"])
    assert after["silicon"] == board["silicon"]
    assert gvs.evaluate(after) == before


def test_write_ack_digests_leaves_undrifted_boards_alone(tmp_path, monkeypatch):
    """The optional legacy writer keeps its bounded default and explicit all mode."""
    boards = [
        {"id": "drifted-one", "drift_ack": "2026-08-31", "drift_ack_digest": "stale",
         "models": ["m.rs"], "silicon": {"last_capture": "2020-01-01"}},
        {"id": "fresh-one", "drift_ack": "2026-08-31", "drift_ack_digest": "stale",
         "models": ["m.rs"], "silicon": {"last_capture": "2099-01-01"}},
    ]
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


# ── Duplicate keys: PyYAML would silently keep the last one ──────────────────

def test_duplicate_drift_ack_is_rejected():
    text = "boards:\n  - id: x\n    drift_ack: 2026-01-01\n    drift_ack: 2026-02-01\n"
    with pytest.raises(SystemExit) as e:
        gvs.load_manifest(text)
    assert "duplicate key 'drift_ack'" in str(e.value)
    assert "line 4" in str(e.value) and "first at line 3" in str(e.value)


def test_same_key_on_different_boards_is_fine():
    text = "boards:\n  - id: x\n    drift_ack: 2026-01-01\n  - id: y\n    drift_ack: 2026-01-01\n"
    assert [b["id"] for b in gvs.load_manifest(text)["boards"]] == ["x", "y"]


def test_committed_manifest_has_no_duplicate_keys():
    gvs.load_manifest(gvs.MANIFEST.read_text())


@pytest.mark.parametrize("key", ["models", "silicon", "drift_ack_digest"])
def test_other_duplicate_evidence_keys_are_rejected(key):
    text = f"boards:\n  - id: demo\n    {key}: null\n    {key}: null\n"
    with pytest.raises(SystemExit, match=f"duplicate key '{key}'"):
        gvs.load_manifest(text)
