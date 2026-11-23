import json
import subprocess
import sys
from pathlib import Path

import board_matrix as bm

REPO_ROOT = Path(__file__).resolve().parents[2]


def _entry(**over):
    base = dict(
        id="demo-board",
        kind="firmware-gate",
        path="examples/iolink-station",
        apt=["gcc-arm-none-eabi"],
        rust_targets=[],
        packs=["stm32cubel4@v1.18.2"],
        submodules="recursive",
        gate=True,
    )
    base.update(over)
    return base


def test_select_pull_request_keeps_only_gates():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "pull_request")] == ["a"]


def test_select_schedule_keeps_all():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "schedule")] == ["a", "b"]


def test_validate_flags_missing_build_script():
    bad = _entry(path="examples/does-not-exist")
    errors = bm.validate([bad], str(REPO_ROOT))
    assert any("ci/build.sh" in e for e in errors)


def test_validate_passes_for_real_iolink_entry():
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    assert bm.validate(entries, str(REPO_ROOT)) == []


def test_to_matrix_joins_lists_to_strings():
    m = bm.to_matrix([_entry(apt=["gcc-arm-none-eabi"], rust_targets=["thumbv6m-none-eabi"], packs=["stm32cubel4@v1.18.2"])])
    inc = m["include"][0]
    assert inc["apt"] == "gcc-arm-none-eabi"
    assert inc["rust_targets"] == "thumbv6m-none-eabi"
    assert inc["packs"] == "stm32cubel4@v1.18.2"
    assert "toolchains" not in inc


def test_cli_emits_json_for_pull_request():
    out = subprocess.check_output(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT)],
        text=True,
    )
    matrix = json.loads(out)
    assert any(e["id"] == "iolink-station-l476" for e in matrix["include"])


def test_iolink_station_installs_newlib_for_nano_specs():
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    iolink = next(e for e in entries if e["id"] == "iolink-station-l476")
    matrix = bm.to_matrix([iolink])
    apt = matrix["include"][0]["apt"].split()
    assert "gcc-arm-none-eabi" in apt
    assert "libnewlib-arm-none-eabi" in apt


def test_cli_exits_nonzero_on_invalid_manifest(tmp_path):
    bad = tmp_path / "boards.yml"
    bad.write_text(
        "boards:\n"
        "  - id: ghost\n"
        "    kind: firmware-gate\n"
        "    path: examples/does-not-exist\n"
        "    gate: true\n"
    )
    result = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT),
         "--manifest", str(bad)],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "ci/build.sh" in result.stderr


def test_cli_exits_nonzero_on_empty_manifest(tmp_path):
    empty = tmp_path / "boards.yml"
    empty.write_text("boards: []\n")
    result = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT),
         "--manifest", str(empty)],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "no boards" in result.stderr


def _all_ungated_manifest(tmp_path):
    """A manifest that is entirely VALID but selects nothing on a gate event.

    Every field the validator checks is correct — real directories, real
    kinds, no duplicate ids — and every entry carries `gate: false`. This is
    one flag flip away from `configs/ci/boards.yml`.
    """
    m = tmp_path / "boards.yml"
    m.write_text(
        "boards:\n"
        "  - id: ungated-firmware\n"
        "    kind: firmware-gate\n"
        "    path: examples/iolink-station\n"
        "    gate: false\n"
        "  - id: ungated-sim\n"
        "    kind: sim-validate\n"
        "    path: examples/esp32c3-blinky\n"
        "    gate: false\n"
    )
    return m


def _run_cli(manifest, event):
    return subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", event, "--repo-root", str(REPO_ROOT),
         "--manifest", str(manifest)],
        capture_output=True, text=True,
    )


def test_cli_exits_nonzero_when_a_gate_event_selects_no_boards(tmp_path):
    """CI-8: an all-`gate: false` manifest must NOT report success.

    `core-board-ci.yml` feeds this script's stdout straight into
    `strategy.matrix` and has no aggregator job. `{"include": []}` skips the
    `board` job entirely, and a skipped matrix job leaves the workflow GREEN
    having compiled no firmware at all. The manifest below is fully valid —
    `validate()` sees nothing wrong with it — so validity alone cannot be the
    gate. The SELECTED list is what the workflow actually runs.
    """
    result = _run_cli(_all_ungated_manifest(tmp_path), "pull_request")
    assert result.returncode != 0, (
        "board_matrix.py reported success for a gate event that selected zero "
        f"boards; stdout={result.stdout!r}"
    )
    assert "selected 0" in result.stderr


def test_cli_allows_an_empty_selection_on_a_non_gate_event(tmp_path):
    """The emptiness check is scoped to GATE_EVENTS.

    `schedule` and `workflow_dispatch` take every entry, so they cannot select
    nothing from a non-empty manifest; the check must not turn into a blanket
    "the matrix may never be empty" rule that fires on some future event that
    legitimately filters everything out.
    """
    result = _run_cli(_all_ungated_manifest(tmp_path), "schedule")
    assert result.returncode == 0, result.stderr
    matrix = json.loads(result.stdout)
    assert [e["id"] for e in matrix["include"]] == ["ungated-firmware", "ungated-sim"]


def test_real_manifest_still_selects_boards_on_a_pull_request():
    """Positive control for the two tests above.

    If the shipped manifest ever stopped selecting anything, the new check
    would redden every PR — which is the intent — but this test names the
    condition directly instead of leaving it to be discovered in CI.
    """
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    for event in sorted(bm.GATE_EVENTS):
        assert bm.select(entries, event), f"{event} selects no boards"



def test_validate_requires_ci_scripts_for_sim_validate():
    bad = _entry(kind="sim-validate", path="examples/does-not-exist")
    errors = bm.validate([bad], str(REPO_ROOT))
    assert any("ci/build.sh" in error for error in errors)
    assert any("ci/test.sh" in error for error in errors)


def _real_entries():
    return bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))


def _push(paths):
    return {e["id"] for e in bm.select(_real_entries(), "push", paths, str(REPO_ROOT))}


def test_trigger_paths_mirror_the_workflow_push_filter():
    import yaml

    wf = yaml.safe_load((REPO_ROOT / ".github/workflows/core-board-ci.yml").read_text())
    on = wf.get("on", wf.get(True))
    assert sorted(on["push"]["paths"]) == sorted(bm.TRIGGER_PATHS)


def test_push_ignores_paths_the_workflow_does_not_trigger_on():
    # A relevant board edit riding with crates/docs changes must not widen to all boards.
    got = _push(["crates/core/src/lib.rs", "docs/x.md", "examples/nucleo-f411re/io-smoke.yaml"])
    assert got == {"nucleo-f411re"}


def test_push_onboarding_only_changes_select_no_board():
    assert _push([
        "configs/chips/onboarding/nrf52840.yaml",
        "configs/systems/onboarding/nrf52840.yaml",
        "examples/renode-onboarding/nrf52840-smoke.yaml",
    ]) == set()


def test_push_chip_change_selects_only_boards_that_read_it():
    got = _push(["configs/chips/stm32f411re.yaml"])
    assert "nucleo-f411re" in got and 0 < len(got) < 5


def test_push_unknown_config_paths_fail_closed():
    gated = {e["id"] for e in _real_entries() if e.get("gate")}
    assert _push(["configs/chips-legacy.yaml"]) == gated
    assert _push(["configs/ci/boards.yml"]) == gated
    assert _push([".github/workflows/core-board-ci.yml"]) == gated


def test_push_new_unmanifested_example_runs_no_board():
    assert _push(["examples/some-new-board-bme280/bme280-smoke.yaml"]) == set()


def _workflow_jobs():
    import yaml

    wf = yaml.safe_load((REPO_ROOT / ".github/workflows/core-board-ci.yml").read_text())
    return wf["jobs"]


def test_setup_exports_has_boards_from_the_emitted_matrix():
    """A scoped push may emit {"include": []}; setup must say so explicitly.

    GitHub Actions fails the run when `strategy.matrix` has an empty
    `include`, so the workflow needs a flag to skip `board` instead.
    """
    setup = _workflow_jobs()["setup"]
    assert setup["outputs"]["has_boards"] == "${{ steps.gen.outputs.has_boards }}"
    gen = next(s for s in setup["steps"] if s.get("id") == "gen")
    assert 'echo "has_boards=$has_boards" >> "$GITHUB_OUTPUT"' in gen["run"]
    assert '["include"]' in gen["run"]


def test_board_job_runs_only_when_the_matrix_has_boards():
    cond = " ".join(_workflow_jobs()["board"]["if"].split())
    assert "needs.setup.outputs.has_boards == 'true'" in cond
    # The schedule fork guard must survive alongside the new condition.
    assert (
        "(github.event_name != 'schedule' || github.repository == 'w1ne/labwired-core')"
        in cond
    )
    assert "&&" in cond


def test_scoped_push_with_no_affected_board_emits_empty_include(tmp_path):
    """The case the has_boards guard exists for: exit 0, empty include."""
    changed = tmp_path / "changed-paths.txt"
    changed.write_text("examples/renode-onboarding/nrf52840-smoke.yaml\n")
    result = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "push", "--repo-root", str(REPO_ROOT),
         "--changed-paths-file", str(changed)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == {"include": []}


def test_unscoped_push_with_no_gated_board_still_fails_closed(tmp_path):
    result = _run_cli(_all_ungated_manifest(tmp_path), "push")
    assert result.returncode != 0
    assert "selected 0" in result.stderr
