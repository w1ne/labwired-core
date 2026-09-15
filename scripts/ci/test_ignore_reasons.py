"""Unit cover for the bare-`#[ignore]` gate.

Every case but two builds a SYNTHETIC tree and points the gate at it. Grading
the real crates/ for a verdict would make these tests a mirror of today's tree:
they would go green for the wrong reason the moment someone documented an
ignore, and they could never express "a bare `#[ignore]` must fail", because
crates/ is (by design) not in that state.

The two cases that do read the real repo assert the gate's own claims about it
-- that the committed baseline still matches the committed tree, and that the
scanner does not count the prose in doc comments -- rather than restating the
current numbers.

The negative cases are the ways this gate could have been useless: an ignore
whose reason wraps across lines counted as bare; prose about `#[ignore]`
counted as an ignore; a baseline slot outliving the entry that earned it.
"""

import json
from pathlib import Path

import ignore_reasons as ir
import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def tree(tmp_path, monkeypatch):
    """A minimal core checkout the gate can grade."""
    (tmp_path / "crates/demo/tests").mkdir(parents=True)
    (tmp_path / "validation").mkdir()
    monkeypatch.setattr(ir, "CORE_ROOT", tmp_path)
    monkeypatch.setattr(ir, "CRATES", tmp_path / "crates")
    monkeypatch.setattr(ir, "BASELINE", tmp_path / "validation/ignore_reasons_baseline.json")
    return tmp_path


def rs(tree, body: str, name: str = "probe.rs"):
    (tree / "crates/demo/tests" / name).write_text(body)


DOCUMENTED = """
#[test]
#[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]
fn h563_uds_over_swd() {}
"""

BARE = """
#[test]
#[ignore]
fn nobody_knows_why() {}
"""


# ── the happy path ───────────────────────────────────────────────────────────


def test_a_documented_ignore_passes(tree):
    rs(tree, DOCUMENTED)
    assert ir.main(["--check"]) == 0


def test_a_bare_ignore_fails(tree):
    rs(tree, BARE)
    assert ir.main(["--check"]) == 1


def test_the_failure_names_the_test_and_the_file(tree, capsys):
    rs(tree, BARE)
    ir.main(["--check"])
    err = capsys.readouterr().err
    assert "nobody_knows_why" in err
    assert "crates/demo/tests/probe.rs" in err


# ── the reason must survive the shapes this tree actually uses ───────────────


def test_a_reason_wrapped_with_a_backslash_is_read(tree):
    """Four ignores in crates/ wrap their reason this way.

    A scanner that only matched the single-line form would report every one of
    them as undocumented -- the inverse of this gate's job, and the kind of
    false red that trains people to bypass it.
    """
    rs(
        tree,
        """
#[test]
#[ignore = "boots the real C3 mask ROM (~150M steps); run with \\
            --release --ignored"]
fn c3_mask_rom_boot() {}
""",
    )
    assert ir.main(["--check"]) == 0
    (entry,) = ir.collect()
    assert entry["reason"].startswith("boots the real C3 mask ROM")
    assert "--release --ignored" in entry["reason"]


def test_prose_about_ignore_in_a_doc_comment_is_not_counted(tree):
    """The measured reason this gate anchors its regex at the line start.

    `grep -c '#\\[ignore\\]'` over this tree reports 84 bare ignores. There are
    28: the other 56 hits are doc comments EXPLAINING why something is or is not
    ignored. A gate built on the grep number would demand reasons for sentences.
    """
    rs(
        tree,
        """
/// This test is `#[ignore]`d because the fixture is 8.5 MB. Its sibling is
/// deliberately NOT `#[ignore]`d -- see the module docs.
#[test]
#[ignore = "needs the 8.5 MB flash image fixture; run with --ignored"]
fn big_fixture() {}
""",
    )
    assert len(ir.collect()) == 1
    assert ir.main(["--check"]) == 0


def test_the_name_is_found_past_intervening_attributes(tree):
    rs(
        tree,
        """
#[cfg(feature = "hw-oracle-nrf52")]
#[ignore = "hw-oracle: requires an SWD-attached nRF52840"]
#[test]
#[should_panic]
fn nrf52_conformance_diff() {}
""",
    )
    (entry,) = ir.collect()
    assert entry["test"] == "nrf52_conformance_diff"


def test_a_placeholder_reason_is_refused(tree, capsys):
    """`#[ignore = "TODO"]` is a bare ignore wearing a hat."""
    rs(
        tree,
        """
#[test]
#[ignore = "TODO"]
fn someday() {}
""",
    )
    assert ir.main(["--check"]) == 1
    assert "floor" in capsys.readouterr().err


# ── the baseline ─────────────────────────────────────────────────────────────


def test_a_baselined_bare_ignore_is_tolerated(tree):
    rs(tree, BARE)
    assert ir.main(["--write"]) == 0
    assert ir.main(["--check"]) == 0


def test_the_baseline_key_survives_the_file_being_renamed(tree):
    """The defect a `file:line` key has, stated as a test.

    generate_ignored_tests.py's first version keyed on `file:line` and went red
    within the hour when an unrelated change added 38 lines above seven entries.
    Moving the whole file must not disturb this baseline.
    """
    rs(tree, BARE, name="probe.rs")
    ir.main(["--write"])
    (tree / "crates/demo/tests/probe.rs").rename(tree / "crates/demo/tests/renamed.rs")
    assert ir.main(["--check"]) == 0


def test_the_baseline_key_survives_lines_being_added_above_it(tree):
    rs(tree, BARE)
    ir.main(["--write"])
    rs(tree, "// a comment\n" * 40 + BARE)
    assert ir.main(["--check"]) == 0


def test_a_second_bare_ignore_is_refused_even_at_the_same_count(tree, capsys):
    """The substitution a plain COUNT ratchet cannot see.

    `generate_ignored_tests.py` allows N bare ignores. Delete one and add
    another and N is still N, so it stays green while the identity of what is
    switched off has changed. Keying on the test catches that.
    """
    rs(tree, BARE)
    ir.main(["--write"])
    rs(
        tree,
        """
#[test]
#[ignore]
fn a_different_undocumented_one() {}
""",
    )
    assert ir.main(["--check"]) == 1
    err = capsys.readouterr().err
    assert "a_different_undocumented_one" in err


def test_a_baseline_entry_that_got_a_reason_must_be_dropped(tree, capsys):
    """An allowance may not outlive its justification.

    Left in, the slot stays open and the next bare `#[ignore]` on a test of that
    name lands in it silently. The baseline only shrinks, and shrinking is a
    committed edit rather than an implicit one.
    """
    rs(tree, BARE)
    ir.main(["--write"])
    rs(
        tree,
        """
#[test]
#[ignore = "measurement probe, not a gate"]
fn nobody_knows_why() {}
""",
    )
    assert ir.main(["--check"]) == 1
    assert "only shrinks" in capsys.readouterr().err
    # ...and --write is the way to record it.
    ir.main(["--write"])
    assert ir.main(["--check"]) == 0


def test_a_missing_baseline_is_not_a_pass(tree):
    """No file means nothing is allowed, not that everything is."""
    rs(tree, BARE)
    assert not ir.BASELINE.exists()
    assert ir.main(["--check"]) == 1


# ── anti-vacuity ─────────────────────────────────────────────────────────────


def test_an_empty_scan_is_an_error_not_a_pass(tree, capsys):
    """A gate that finds nothing and ticks is the bug it exists to catch.

    crates/ always holds `#[ignore]` attributes, so zero means the layout moved
    or the regex broke -- never "all clear".
    """
    assert ir.main(["--check"]) == 2
    assert "wrong path" in capsys.readouterr().err


# ── against the real repository ──────────────────────────────────────────────


def test_the_committed_baseline_matches_the_committed_tree():
    """Whatever the tree is, the committed baseline must describe it exactly."""
    assert ir.main(["--check"]) == 0


def test_the_committed_baseline_is_valid_json_with_the_expected_shape():
    if not ir.BASELINE.exists():
        pytest.skip("no baseline committed")
    data = json.loads(ir.BASELINE.read_text())
    assert set(data) == {"entries"}
    for key, info in data["entries"].items():
        assert len(key) == 16, key
        assert {"crate", "test", "file"} <= set(info)
        assert (REPO_ROOT / info["file"]).exists(), info["file"]
