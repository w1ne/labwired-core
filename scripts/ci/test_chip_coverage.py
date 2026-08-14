"""Unit cover for the chip-coverage gate.

Every case builds a SYNTHETIC tree and points the gate at it with
`--repo-root`. Grading the real checkout here would make these tests a mirror
of today's tree: they would go green for the wrong reason the moment someone
edited a workflow, and they could never express "a chip with no gate must
fail", because the real tree is (by design) not in that state.

The one case that does read the real repo is the last one, and it asserts the
committed ledger and the committed workflows agree — which is the gate's own
claim, not a restatement of it.
"""

import subprocess
import sys
import textwrap
from pathlib import Path

import chip_coverage as cc

REPO_ROOT = Path(__file__).resolve().parents[2]
GATE = REPO_ROOT / "scripts/ci/chip_coverage.py"


def build_tree(root: Path, *, chips, arduino_boards, matrix_ids, ledger, scripts=()):
    """Write the smallest tree the gate can grade.

    scripts: iterable of (workflow_name, script_rel, system_rel, chip) — a
    workflow that runs `labwired test` on a script whose system names a chip.
    """
    (root / "configs/chips").mkdir(parents=True)
    for chip in chips:
        (root / f"configs/chips/{chip}.yaml").write_text(f"name: {chip}\n")

    (root / "validation/arduino-matrix").mkdir(parents=True)
    boards = "boards:\n" + "".join(
        f"  - id: {bid}\n    chip: {chip}\n" for bid, chip in arduino_boards
    )
    (root / "validation/arduino-matrix/boards.yaml").write_text(boards)

    (root / ".github/workflows").mkdir(parents=True)
    listed = "".join(f"          - {bid}\n" for bid in matrix_ids)
    (root / ".github/workflows/core-arduino-matrix-smoke.yml").write_text(
        "jobs:\n  arduino-matrix:\n    strategy:\n      matrix:\n        board:\n" + listed
    )

    for name, script_rel, system_rel, chip in scripts:
        script = root / script_rel
        script.parent.mkdir(parents=True, exist_ok=True)
        rel_to_system = Path("../" * len(Path(script_rel).parent.parts)) / system_rel
        script.write_text(
            f'inputs:\n  firmware: "./fw.elf"\n  system: "{rel_to_system}"\n'
        )
        system = root / system_rel
        system.parent.mkdir(parents=True, exist_ok=True)
        system.write_text(f'chip: "{chip}"\n')
        (root / f".github/workflows/{name}").write_text(
            textwrap.dedent(f"""\
                jobs:
                  smoke:
                    steps:
                      - run: labwired test --script {script_rel}
                """)
        )

    (root / "configs/ci").mkdir(parents=True, exist_ok=True)
    # `ledger=None` builds a tree with NO ledger file — the state the repo is in
    # once every chip is covered.
    if ledger is not None:
        (root / "configs/ci/chip-coverage.yaml").write_text(ledger)


def run_gate(root: Path):
    proc = subprocess.run(
        [sys.executable, str(GATE), "--repo-root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode, proc.stdout + proc.stderr


EMPTY_LEDGER = "max_uncovered: 0\nuncovered: {}\n"


def test_no_ledger_file_fails_closed_on_an_uncovered_chip(tmp_path):
    """Deleting the ledger must make the gate stricter, not quieter.

    The file goes when the last gap does, so its absence has to mean "nothing is
    declared" — never "nothing is checked". A chip no gate runs must still be an
    error with no file present, or deleting it would be the way around the gate.
    """
    build_tree(
        tmp_path,
        chips=["stm32f103", "rp2350"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger=None,
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "rp2350" in out


def test_no_ledger_file_passes_when_every_chip_is_covered(tmp_path):
    """And with everything covered, the absent file is simply the end state."""
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger=None,
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_a_chip_in_the_matrix_is_covered(tmp_path):
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_a_chip_with_no_gate_fails(tmp_path):
    build_tree(
        tmp_path,
        chips=["stm32f103", "stm32f042"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "stm32f042" in out and "NO CI gate executes firmware" in out


def test_a_boards_yaml_entry_the_workflow_never_names_is_not_coverage(tmp_path):
    """The h735 shape: declared in the manifest, run by nothing."""
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=[],  # workflow lists no boards at all
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "does not name it, so nothing runs it" in out


def test_a_script_a_workflow_runs_counts_as_coverage(tmp_path):
    build_tree(
        tmp_path,
        chips=["nrf54l15"],
        arduino_boards=[],
        matrix_ids=[],
        ledger=EMPTY_LEDGER,
        scripts=[("smoke.yml", "examples/nrf54l15-dk/io-smoke.yaml", "configs/systems/nrf54l15dk.yaml", "nrf54l15")],
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_a_script_no_workflow_runs_is_not_coverage(tmp_path):
    build_tree(
        tmp_path,
        chips=["nrf54l15"],
        arduino_boards=[],
        matrix_ids=[],
        ledger=EMPTY_LEDGER,
        scripts=[("smoke.yml", "examples/nrf54l15-dk/io-smoke.yaml", "configs/systems/nrf54l15dk.yaml", "nrf54l15")],
    )
    # Blank the workflow that referenced the script; the script and its system
    # stay exactly where they were.
    (tmp_path / ".github/workflows/smoke.yml").write_text("jobs: {}\n")
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "nrf54l15" in out and "NO CI gate executes firmware" in out


def test_a_declared_gap_passes_with_a_reason(tmp_path):
    build_tree(
        tmp_path,
        chips=["rp2350"],
        arduino_boards=[],
        matrix_ids=[],
        ledger="max_uncovered: 1\nuncovered:\n  rp2350:\n    kind: pending\n    reason: no smoke script yet\n",
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_a_gap_without_a_reason_fails(tmp_path):
    build_tree(
        tmp_path,
        chips=["rp2350"],
        arduino_boards=[],
        matrix_ids=[],
        ledger="max_uncovered: 1\nuncovered:\n  rp2350:\n    kind: pending\n    reason: ''\n",
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "needs a concrete `reason`" in out


def test_an_unknown_kind_fails(tmp_path):
    build_tree(
        tmp_path,
        chips=["rp2350"],
        arduino_boards=[],
        matrix_ids=[],
        ledger="max_uncovered: 1\nuncovered:\n  rp2350:\n    kind: someday\n    reason: later\n",
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "`kind` must be one of" in out


def test_a_covered_chip_may_not_keep_its_excuse(tmp_path):
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger="max_uncovered: 1\nuncovered:\n  stm32f103:\n    kind: pending\n    reason: stale\n",
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "is now proven by" in out


def test_a_deleted_chip_may_not_linger_in_the_ledger(tmp_path):
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger="max_uncovered: 1\nuncovered:\n  stm32f042:\n    kind: waived\n    reason: gone\n",
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "not a shipped chip" in out


def test_the_ceiling_must_track_the_gap_exactly(tmp_path):
    """Closing a gap forces the ceiling down, so it can never grow back quietly."""
    ledger = (
        "max_uncovered: 2\nuncovered:\n  rp2350:\n    kind: pending\n    reason: no script yet\n"
    )
    build_tree(
        tmp_path,
        chips=["rp2350"],
        arduino_boards=[],
        matrix_ids=[],
        ledger=ledger,
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert "Lower `max_uncovered:` to 1" in out


def test_ci_fixture_descriptors_are_not_graded(tmp_path):
    build_tree(
        tmp_path,
        chips=["stm32f103", "ci-fixture-riscv"],
        arduino_boards=[("stm32f103", "stm32f103")],
        matrix_ids=["stm32f103"],
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_the_committed_tree_passes_its_own_gate():
    """The repo's real ledger must match the repo's real workflows."""
    cc.set_root(REPO_ROOT)
    _, uncovered, errors = cc.evaluate()
    assert errors == [], errors
    # The ledger exists exactly while there is something to declare. An empty
    # one is a page of instructions for a list with nothing in it, and it reads
    # like debt that is still owed; a missing one is the honest end state, and
    # it fails closed — with no entries, any uncovered chip is an error.
    if uncovered:
        assert cc.ledger_path().is_file(), (
            f"{sorted(uncovered)} are uncovered but there is no ledger to declare them in"
        )
    else:
        assert not cc.ledger_path().is_file(), (
            "nothing is uncovered — delete configs/ci/chip-coverage.yaml rather "
            "than keeping an empty one"
        )
