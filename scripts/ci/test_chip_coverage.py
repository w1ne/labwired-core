"""Unit cover for the chip-coverage gate.

Every case builds a SYNTHETIC tree and points the gate at it with
`--repo-root`. Grading the real checkout here would make these tests a mirror
of today's tree: they would go green for the wrong reason the moment someone
edited a workflow, and they could never express "a chip with no gate must
fail", because the real tree is (by design) not in that state.

The cases that read the real repo assert that committed workflows, coverage
ledgers, and the exact-system smoke contract agree — claims of the gate itself,
not restatements of synthetic cases.
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

    arduino_boards: (id, chip) pairs, or (id, chip, extra_yaml) where the third
    element is appended verbatim under the entry — how the external-lane tests
    below attach `external_compile:` / `lane:`.
    """
    (root / "configs/chips").mkdir(parents=True)
    for chip in chips:
        (root / f"configs/chips/{chip}.yaml").write_text(f"name: {chip}\n")

    (root / "validation/arduino-matrix").mkdir(parents=True)
    boards = "boards:\n" + "".join(
        f"  - id: {b[0]}\n    chip: {b[1]}\n" + (b[2] if len(b) > 2 else "")
        for b in arduino_boards
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


# ── external-compile lanes ────────────────────────────────────────────────
#
# One board cannot compile in this repo at all: BRD2709A's Arduino core is
# hand-written and lives in the labwired monorepo, so `external_compile:` says
# so and `lane:` names the workflow that does run it. Three rules, one for each
# way that could become a hole.

EXTERNAL = '    external_compile:\n      target: "silabs-arduino:v"\n'
LANE = '    lane: "labwired/.github/workflows/x.yml"\n'


def test_an_external_compile_board_with_a_lane_is_coverage(tmp_path):
    build_tree(
        tmp_path,
        chips=["efr32mg26"],
        arduino_boards=[("brd2709a", "efr32mg26", EXTERNAL + LANE)],
        matrix_ids=[],  # deliberately absent from THIS repo's matrix
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_an_external_compile_board_with_no_lane_is_a_hole(tmp_path):
    """Without a lane, nothing anywhere runs it — the exact failure this gate is for."""
    build_tree(
        tmp_path,
        chips=["efr32mg26"],
        arduino_boards=[("brd2709a", "efr32mg26", EXTERNAL)],
        matrix_ids=[],
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code != 0
    assert "names no `lane:`" in out


def test_an_external_compile_board_in_this_repos_matrix_is_refused(tmp_path):
    """The job could only ever report toolchain_missing, and would burn a runner doing it."""
    build_tree(
        tmp_path,
        chips=["efr32mg26"],
        arduino_boards=[("brd2709a", "efr32mg26", EXTERNAL + LANE)],
        matrix_ids=["brd2709a"],
        ledger=EMPTY_LEDGER,
    )
    code, out = run_gate(tmp_path)
    assert code != 0
    assert "toolchain_missing" in out


def test_a_lane_alone_does_not_excuse_an_ordinary_board(tmp_path):
    """`lane:` is only meaningful with `external_compile:`.

    Otherwise any board could be waved past the matrix by adding one line, which
    would turn the rule above into a suggestion.
    """
    build_tree(
        tmp_path,
        chips=["stm32f103"],
        arduino_boards=[("stm32f103", "stm32f103", LANE)],
        matrix_ids=[],
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


def test_a_matrix_system_override_proves_each_exact_chip_only_when_run(tmp_path):
    chips = ["stm32f103c4", "stm32f401cb"]
    build_tree(
        tmp_path,
        chips=chips,
        arduino_boards=[],
        matrix_ids=[],
        ledger=EMPTY_LEDGER,
    )

    script_rel = "examples/ci/arm-spin-smoke.yaml"
    script = tmp_path / script_rel
    script.parent.mkdir(parents=True)
    script.write_text(
        'inputs:\n  firmware: "./fw.elf"\n  system: "./unused-system.yaml"\n'
    )

    systems = [f"configs/systems/proteus/{chip}.yaml" for chip in chips]
    for chip, system_rel in zip(chips, systems, strict=True):
        system = tmp_path / system_rel
        system.parent.mkdir(parents=True, exist_ok=True)
        system.write_text(f'chip: "../../chips/{chip}.yaml"\n')

    workflow = tmp_path / ".github/workflows/core-coverage-matrix-smoke.yml"
    workflow.write_text(
        textwrap.dedent(
            f'''\
            jobs:
              coverage-matrix-smoke:
                strategy:
                  matrix:
                    include:
                      - id: exact-stm32-catalog
                        script: {script_rel}
                        exact_systems:
            '''
        )
        + "".join(f"              - {system}\n" for system in systems)
        + '''    steps:
      - name: Run exact variants
        if: matrix.id == 'exact-stm32-catalog'
        env:
          EXACT_SYSTEMS: "${{ join(matrix.exact_systems, ' ') }}"
        run: |
          for system in $EXACT_SYSTEMS; do
                ./target/release/labwired test --script "${{ matrix.script }}" --system "$system" --output-dir "$out" --no-uart-stdout
          done
'''
    )

    valid_workflow = workflow.read_text()
    code, out = run_gate(tmp_path)
    assert code == 0, out

    disabled_job = valid_workflow.replace(
        "  coverage-matrix-smoke:\n",
        "  coverage-matrix-smoke:\n    if: false\n",
        1,
    )
    workflow.write_text(disabled_job)
    parsed_workflow = cc.load_yaml(workflow)
    assert parsed_workflow["jobs"]["coverage-matrix-smoke"]["if"] is False
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    job_may_ignore_failure = valid_workflow.replace(
        "  coverage-matrix-smoke:\n",
        "  coverage-matrix-smoke:\n    continue-on-error: true\n",
        1,
    )
    workflow.write_text(job_may_ignore_failure)
    parsed_workflow = cc.load_yaml(workflow)
    assert parsed_workflow["jobs"]["coverage-matrix-smoke"]["continue-on-error"] is True
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    run_line = next(
        line for line in valid_workflow.splitlines() if line.strip() == "run: |"
    )
    run_indent = len(run_line) - len(run_line.lstrip())
    step_may_ignore_failure = valid_workflow.replace(
        run_line,
        f"{' ' * run_indent}continue-on-error: true\n{run_line}",
        1,
    )
    workflow.write_text(step_may_ignore_failure)
    parsed_workflow = cc.load_yaml(workflow)
    assert (
        parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["continue-on-error"]
        is True
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    help_workflow = valid_workflow.replace(
        "--no-uart-stdout", "--no-uart-stdout --help"
    )
    workflow.write_text(help_workflow)
    parsed_workflow = cc.load_yaml(workflow)
    help_run = parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["run"]
    assert "--help" in help_run
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    # Keep the loop and its arguments in place, but make the apparent CLI call
    # inert. Argument text in an echo or a comment is not evidence that any
    # firmware ran against these exact systems.
    for inert_prefix in ("echo ", "# "):
        workflow.write_text(
            valid_workflow.replace(
                "./target/release/labwired test --script",
                f"{inert_prefix}./target/release/labwired test --script",
            )
        )
        code, out = run_gate(tmp_path)
        assert code == 1, out
        assert all(chip in out for chip in chips)
        assert "NO CI gate executes firmware" in out

    # `--system` inside another option's quoted value is not a system override.
    quoted_argument = valid_workflow.replace(
        ' --system "$system"',
        " --output-dir '--system \"$system\"'",
    )
    workflow.write_text(quoted_argument)
    parsed_workflow = cc.load_yaml(workflow)
    quoted_run = parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["run"]
    assert "--output-dir '--system \"$system\"'" in quoted_run
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    wrapper_workflow = valid_workflow.replace(
        "./target/release/labwired", "./tmp/bin/labwired"
    )
    workflow.write_text(wrapper_workflow)
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    # A matching invocation inside a filter can skip exact systems. Credit only
    # a straight-line loop body that invokes the CLI on every iteration.
    command_line = next(
        line for line in valid_workflow.splitlines() if "labwired test --script" in line
    )
    command_indent = command_line[: len(command_line) - len(command_line.lstrip())]
    filtered_workflow = valid_workflow.replace(
        command_line.strip(),
        f'if [ "$system" = "never" ]; then\n{command_indent}  {command_line.strip()}',
    )
    loop_close = filtered_workflow.rfind("done")
    loop_close_line = filtered_workflow.rfind("\n", 0, loop_close) + 1
    filtered_workflow = (
        filtered_workflow[:loop_close_line]
        + f"{command_indent}fi\n"
        + filtered_workflow[loop_close_line:]
    )
    workflow.write_text(filtered_workflow)
    parsed_workflow = cc.load_yaml(workflow)
    filtered_run = parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["run"]
    assert 'if [ "$system" = "never" ]' in filtered_run
    assert "labwired test --script" in filtered_run
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    # Compound shell filters can skip an iteration without using a multiline
    # `if` block, so they must also be rejected.
    compound_guarded = valid_workflow.replace(
        "./target/release/labwired test --script",
        '[[ "$system" != "never" ]] && continue\n'
        '            ./target/release/labwired test --script',
    )
    workflow.write_text(compound_guarded)
    parsed_workflow = cc.load_yaml(workflow)
    compound_run = parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["run"]
    assert '[[ "$system" != "never" ]] && continue' in compound_run
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    # `exec` replaces the shell successfully, preventing the later CLI line
    # from running even though it is present in the loop text.
    exec_workflow = valid_workflow.replace(
        "./target/release/labwired test --script",
        "exec true\n            ./target/release/labwired test --script",
    )
    workflow.write_text(exec_workflow)
    parsed_workflow = cc.load_yaml(workflow)
    exec_run = parsed_workflow["jobs"]["coverage-matrix-smoke"]["steps"][0]["run"]
    assert "exec true" in exec_run
    assert "labwired test --script" in exec_run
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    workflow.write_text(valid_workflow.replace(' --system "$system"', ""))
    code, out = run_gate(tmp_path)
    assert code == 1
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out

    workflow.write_text(
        valid_workflow.replace(
            "matrix.id == 'exact-stm32-catalog'",
            "matrix.id == 'different-matrix-cell'",
        )
    )
    code, out = run_gate(tmp_path)
    assert code == 1
    assert all(chip in out for chip in chips)
    assert "NO CI gate executes firmware" in out


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


def test_committed_debt_records_samd21_geometry_and_rv1103_gaps():
    """Exact SAMD21 alternates stay pending until each gets a firmware smoke."""
    cc.set_root(REPO_ROOT)
    entries, ceiling = cc.load_ledger()
    samd21_alternates = {
        "atsamd21e15a", "atsamd21e16a", "atsamd21e17a", "atsamd21e18a",
        "atsamd21g15a", "atsamd21g16a", "atsamd21g17a",
        "atsamd21j15a", "atsamd21j16a", "atsamd21j17a", "atsamd21j18a",
    }
    assert ceiling == len(samd21_alternates) + 1
    assert set(entries) == samd21_alternates | {"rv1103"}
    for chip in samd21_alternates:
        assert entries[chip]["kind"] == "pending"
        assert "no CLI firmware smoke boots this exact chip" in entries[chip]["reason"]
    assert entries["rv1103"]["kind"] == "waived"
    reason = entries["rv1103"]["reason"]
    assert "No CI gate qualifies full RV1103 platform coverage" in reason
    assert "generic Cortex-A7" in reason
    descriptor = cc.load_yaml(cc.chip_dir() / "rv1103.yaml")
    assert descriptor["core"] == "cortex-a7"
    assert descriptor["peripherals"] == []


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


def test_proteus_stm32_exact_catalog_skips_the_fixture_smoke():
    """The exact row must reach its per-system override without the generic fixture run."""
    workflow = (REPO_ROOT / ".github/workflows/core-coverage-matrix-smoke.yml").read_text()
    generic_step = workflow.split("      - name: Run matrix smoke\n", 1)[1].split(
        "      - name:", 1
    )[0]
    exact_step = workflow.split(
        "      - name: Run exact Proteus STM32 catalog variants\n", 1
    )[1].split("      - name:", 1)[0]

    assert "if: matrix.id != 'proteus-stm32-exact-catalog'" in generic_step
    assert "if: matrix.id == 'proteus-stm32-exact-catalog'" in exact_step
