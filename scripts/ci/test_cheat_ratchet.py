"""Unit cover for the CHEAT ratchet.

Every case builds a SYNTHETIC tree and points the gate at it. Grading the real
crates/ for a VERDICT would make these tests a mirror of today's surface: they
would go green for the wrong reason the moment someone marked a cheat, and they
could never express "a marker with no `real:` clause must fail", because the
tree is (by design) not in that state.

Two cases do read the real repo, and they assert the gate's own claims about it
— that the committed baseline covers the committed markers, and that the
taxonomy parses — rather than restating either.

Each negative case is a defect this gate found on its first run against main.
"""

import json
from datetime import date
from pathlib import Path

import cheat_ratchet as cr
import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]

TAXONOMY = """
## Marking

// CHEAT(<CATEGORY>): <what is faked> — real: <what silicon does>

//! CHEAT(<CATEGORY>, module): <what every fn here fakes> — real: <what silicon does>

| Marker | Meaning | Exit |
|---|---|---|
| `CHEAT(NOP)` | A function is replaced by a constant return. | Model it. |
| `CHEAT(STUB)` | A peripheral is faked as plain RAM. | Implement the registers. |
| `CHEAT(THUNK)` | Umbrella for a module of thunks of both kinds. | Reduce it. |
"""


@pytest.fixture
def tree(tmp_path, monkeypatch):
    """A minimal core checkout the gate can grade."""
    (tmp_path / "crates/demo/src").mkdir(parents=True)
    (tmp_path / "validation").mkdir()
    (tmp_path / "FIDELITY.md").write_text(TAXONOMY)
    monkeypatch.setattr(cr, "CORE_ROOT", tmp_path)
    monkeypatch.setattr(cr, "FIDELITY", tmp_path / "FIDELITY.md")
    monkeypatch.setattr(cr, "BASELINE", tmp_path / "validation/cheat_baseline.json")
    monkeypatch.setattr(cr, "VALIDATION_DIR", tmp_path / "validation")
    return tmp_path


def rs(tree, body: str, name: str = "lib.rs"):
    (tree / "crates/demo/src" / name).write_text(body)


def seed(tree):
    """Accept whatever is currently marked, so later cases test one thing."""
    markers, _ = cr.collect_markers()
    cr.write_baseline(markers)


HONEST = """
// CHEAT(NOP): returns 0 for the call — real: the function's actual effect runs.
pub fn thing() {}
"""


def test_the_taxonomy_is_parsed_not_hardcoded(tree):
    assert cr.documented_categories() == {"NOP", "STUB", "THUNK"}


def test_an_honest_marked_tree_passes(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    assert cr.main(["--check"]) == 0


def test_an_undocumented_category_fails(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    rs(tree, HONEST.replace("CHEAT(NOP)", "CHEAT(GUESS)"))
    assert cr.main(["--check"]) == 1
    assert "is not a category FIDELITY.md documents" in capsys.readouterr().err


def test_a_marker_without_a_real_clause_fails(tree, capsys):
    rs(tree, "// CHEAT(NOP): returns 0 for the call.\npub fn thing() {}\n")
    seed(tree)
    assert cr.main(["--check"]) == 1
    assert "has no `real:` clause" in capsys.readouterr().err


def test_a_new_cheat_fails_but_a_removed_one_does_not(tree, capsys):
    """The ratchet, both directions. It must fall freely and rise only on review."""
    rs(tree, HONEST)
    seed(tree)

    rs(tree, HONEST + "\n// CHEAT(STUB): faked as RAM — real: model the registers.\npub fn two() {}\n")
    assert cr.main(["--check"]) == 1, "a new cheat must fail"
    assert "NEW cheat" in capsys.readouterr().err

    rs(tree, "")
    assert cr.main(["--check"]) == 0, "removing every cheat must PASS — the count may always fall"


def test_the_key_survives_a_move_and_a_rename(tree):
    """Content-keyed, because file:line rots — which is how this repo got here."""
    rs(tree, HONEST)
    seed(tree)
    (tree / "crates/demo/src/lib.rs").unlink()
    rs(tree, "\n\n\n" + HONEST.replace("pub fn thing", "pub fn renamed"), name="moved.rs")
    assert cr.main(["--check"]) == 0, "a move + rename must not read as a new cheat"


def test_editing_what_is_faked_does_read_as_new(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    rs(tree, HONEST.replace("returns 0 for the call", "returns a fabricated pointer"))
    assert cr.main(["--check"]) == 1
    assert "NEW cheat" in capsys.readouterr().err


def test_fidelity_naming_a_path_that_does_not_exist_fails(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    (tree / "FIDELITY.md").write_text(TAXONOMY + "\nSee `crates/demo/src/gone.rs` for the list.\n")
    assert cr.main(["--check"]) == 1
    assert "which does not exist" in capsys.readouterr().err


def test_a_citation_to_a_marker_not_in_this_file_fails(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    rs(tree, "// see the CHEAT(STUB) above that fakes it\npub fn other() {}\n", name="other.rs")
    assert cr.main(["--check"]) == 1
    assert "cites CHEAT(STUB), which is not declared in this file" in capsys.readouterr().err


def test_a_validation_citation_landing_off_a_marker_line_fails(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    (tree / "validation/exercise.yaml").write_text('- {ev: "demo/src/lib.rs:99 (CHEAT(NOP))"}\n')
    assert cr.main(["--check"]) == 1
    assert "no marker is on that line" in capsys.readouterr().err


def test_a_prose_mention_is_not_a_cheat(tree):
    """`CHEAT(...)` without a colon is a citation. Counting those is why "25
    markers" was never reproducible: three of the 25 on main were prose."""
    rs(tree, HONEST + '\nconst DOC: &str = "write CHEAT(NOP): like this";\n')
    seed(tree)
    markers, _ = cr.collect_markers()
    assert len(markers) == 1, [m["clause"] for m in markers]


def test_an_expired_waiver_fails(tree, capsys):
    rs(tree, HONEST)
    seed(tree)
    base = json.loads((tree / "validation/cheat_baseline.json").read_text())
    base["waived"] = [{"key": "abc", "reason": "probe on order", "expires": "2020-01-01"}]
    (tree / "validation/cheat_baseline.json").write_text(json.dumps(base))
    assert cr.main(["--check"]) == 1
    assert "expired 2020-01-01" in capsys.readouterr().err


def test_a_live_waiver_covers_a_cheat_that_is_not_in_the_baseline(tree):
    rs(tree, HONEST)
    seed(tree)
    markers, _ = cr.collect_markers()
    key = cr.key_of(markers[0])
    (tree / "validation/cheat_baseline.json").write_text(
        json.dumps({"entries": {}, "waived": [{"key": key, "reason": "tracked", "expires": "2999-01-01"}]})
    )
    assert cr.main(["--check"]) == 0


def test_module_scope_is_recorded(tree):
    rs(tree, "//! CHEAT(THUNK, module): every fn fakes it — real: the code runs.\n")
    markers, errors = cr.collect_markers()
    assert errors == []
    assert markers[0]["module_scope"] is True


# ── The real tree: the gate's own claims about it, not a restatement ─────────


def test_the_committed_baseline_covers_the_committed_markers():
    """This is the gate. If it fails, main is red and that is the intent."""
    assert cr.main(["--check"]) == 0


def test_the_real_taxonomy_parses_and_covers_every_marker_in_use():
    documented = cr.documented_categories()
    assert len(documented) >= 7, documented
    markers, _ = cr.collect_markers()
    assert markers, "no markers found — the parser stopped seeing the surface it grades"
    assert {m["category"] for m in markers} <= documented
