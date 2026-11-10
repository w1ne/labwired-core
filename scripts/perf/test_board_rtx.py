from pathlib import Path
from types import SimpleNamespace

import pytest

import board_rtx as br


def test_cpu_hz_requires_positive_integer():
    assert br.cpu_hz({"cpu_hz": 48_000_000}) == 48_000_000
    for value in (None, 0, -1, 1.5, "48000000"):
        with pytest.raises(ValueError):
            br.cpu_hz({"cpu_hz": value})


def test_run_once_requires_exact_batched_proof(monkeypatch):
    times = iter((10.0, 10.25))
    monkeypatch.setattr(br.time, "perf_counter", lambda: next(times))
    monkeypatch.setattr(
        br.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(
            returncode=0,
            stdout="",
            stderr=(
                "[batched] instructions=48000000 batches=93750 "
                "steps_per_batch=512.00 tick_interval=512\n"
            ),
        ),
    )
    wall, width = br.run_once(Path("labwired"), "atsamd21", Path("spin.elf"), 48_000_000)
    assert wall == pytest.approx(0.25)
    assert width == pytest.approx(512.0)


def test_run_once_rejects_partial_work(monkeypatch):
    times = iter((1.0, 2.0))
    monkeypatch.setattr(br.time, "perf_counter", lambda: next(times))
    monkeypatch.setattr(
        br.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(
            returncode=0,
            stdout="",
            stderr=(
                "[batched] instructions=47 batches=1 "
                "steps_per_batch=47.00 tick_interval=512\n"
            ),
        ),
    )
    with pytest.raises(br.bp.ModeNotTakenError):
        br.run_once(Path("labwired"), "atsamd21", Path("spin.elf"), 48)


DECLARED = {"arch": "arm", "flash": {"base": 0, "size": 0}, "ram": {"base": 0, "size": "64MiB"}, "peripherals": []}


def test_fixture_could_run_requires_a_matching_fixture(monkeypatch):
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: None)
    assert not br.fixture_could_run(DECLARED)
    assert not br.fixture_could_run({**DECLARED, "peripherals": [{"id": "uart0"}]})
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: "cortex-m")
    assert br.fixture_could_run(DECLARED)


def test_real_chip_tree_has_no_waiver_for_a_runnable_perf_fixture():
    """A coverage-only pass must not conceal a guaranteed RTx startup failure."""
    chips = br.bp.discover_chips()
    _, waived = br.bp.plan_coverage(chips)
    unprovable = sorted(board for board in waived if br.fixture_could_run(chips[board]))
    assert not unprovable, f"runnable chips must be measured, not waived: {unprovable}"


def _run_main(monkeypatch, tmp_path, chips, waived, status, fixture=None):
    cli = tmp_path / "labwired"
    cli.write_text("")
    monkeypatch.setattr(br.bp, "discover_chips", lambda: chips)
    monkeypatch.setattr(br.bp, "plan_coverage", lambda c: ({}, waived))
    monkeypatch.setattr(br.bp, "build_fixtures", lambda needed: ({}, {}))
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: fixture)
    monkeypatch.setattr(
        "sys.argv", ["board_rtx.py", "--cli", str(cli), "--status-json", str(status)]
    )
    return br.main()


def test_a_waiver_on_a_fixture_backed_chip_still_refuses(monkeypatch, tmp_path, capsys):
    runnable = {**DECLARED, "peripherals": [{"id": "uart0"}], "flash": {"base": 0, "size": 4096}}
    status = tmp_path / "status.json"
    code = _run_main(
        monkeypatch, tmp_path, {"a": runnable}, {"a": "because"}, status, "cortex-m"
    )
    assert code == 1
    assert "waivers remain on runnable chips: a" in capsys.readouterr().err
    assert not status.exists()


def test_a_platform_without_a_fixture_is_reported_not_refused(monkeypatch, tmp_path, capsys):
    platform = {**DECLARED, "peripherals": [{"id": "uart0"}], "flash": {"base": 0, "size": 4096}}
    status = tmp_path / "status.json"
    code = _run_main(monkeypatch, tmp_path, {"p": platform}, {"p": "no fixture"}, status)
    assert code == 0
    out = capsys.readouterr().out
    assert "not measured (no perf fixture): p: no fixture" in out
    assert "1 waived chip(s) not measured" in out
    import json
    assert json.loads(status.read_text())["not_measured"] == {"p": "no fixture"}


def test_a_declared_only_waiver_is_reported_not_refused(monkeypatch, tmp_path, capsys):
    status = tmp_path / "status.json"
    code = _run_main(monkeypatch, tmp_path, {"d": DECLARED}, {"d": "no platform"}, status)
    assert code == 0
    out = capsys.readouterr().out
    assert "not measured (no perf fixture): d: no platform" in out
    assert "1 waived chip(s) not measured" in out
    import json
    assert json.loads(status.read_text())["not_measured"] == {"d": "no platform"}


def test_boards_argument_limits_coverage_and_measurement(monkeypatch, tmp_path):
    cli = tmp_path / "labwired"
    cli.write_text("")
    chips = {
        "keep": {**DECLARED, "cpu_hz": 48_000_000},
        "drop": {**DECLARED, "cpu_hz": 48_000_000},
    }
    seen_coverage = []
    seen_measurements = []

    monkeypatch.setattr(br.bp, "discover_chips", lambda: chips)

    def plan_coverage(selected):
        seen_coverage.append(set(selected))
        return {"keep": "cortex-m"}, {}

    monkeypatch.setattr(br.bp, "plan_coverage", plan_coverage)
    monkeypatch.setattr(
        br.bp,
        "build_fixtures",
        lambda fixtures: ({fixture: tmp_path / f"{fixture}.elf" for fixture in fixtures}, {}),
    )

    def run_once(_cli, board, _firmware, _steps):
        seen_measurements.append(board)
        return 0.25, 1024.0

    monkeypatch.setattr(br, "run_once", run_once)
    monkeypatch.setattr(
        "sys.argv",
        ["board_rtx.py", "--cli", str(cli), "--boards", "keep"],
    )

    assert br.main() == 0
    assert seen_coverage == [{"keep"}]
    assert seen_measurements == ["keep"] * 3


# --- realistic workload RTF (board_rtf.py) ---------------------------------

import json

import board_rtf as rtf


def _row(workload="uno_feeder", chip="atmega328p", value=0.1):
    return {"workload": workload, "chip": chip, "rtf": value}


def test_rtf_is_guest_seconds_over_wall_seconds():
    assert rtf.rtf(16_000_000, 16_000_000, 4.0) == pytest.approx(0.25)
    for hz, wall in ((0, 1.0), (16_000_000, 0.0)):
        with pytest.raises(ValueError):
            rtf.rtf(1, hz, wall)


def test_rtf_workloads_have_committed_inputs():
    assert {w.chip for w in rtf.WORKLOADS} >= {"atmega328p", "rp2040", "esp32c3", "stm32f411ceu6"}
    for w in rtf.WORKLOADS:
        for name in ("firmware.elf", "system.yaml", f"{w.id}.ino"):
            assert (w.dir / name).is_file(), f"{w.id}: missing {name}"


def test_rtf_refuses_a_run_that_stopped_early():
    w = rtf.WORKLOADS[0]
    ok = rtf.parse_result(w, {"status": "pass", "stop_reason": "max_cycles"}, 2.0, 0)
    assert ok["cycles"] == round(w.cpu_hz * w.guest_s)
    for result, code in (
        ({"status": "pass", "stop_reason": "assertions_passed"}, 0),
        ({"status": "fail", "stop_reason": "max_cycles"}, 1),
    ):
        with pytest.raises(RuntimeError):
            rtf.parse_result(w, result, 2.0, code)


def test_rtf_gate_is_host_independent_and_flags_a_20_percent_slowdown():
    base = {"uno_feeder": {"index": 0.1}}
    # Same engine on a host twice as fast: both RTF and spin RTx double.
    assert rtf.judge([_row(value=0.2)], {"atmega328p": 2.0}, base, 0.10) == []
    # Synthetic 20% slowdown of the workload alone marks the gate red at a
    # 10% tolerance, and a 30% slowdown at the default one.
    assert rtf.judge([_row(value=0.07)], {"atmega328p": 1.0}, base, rtf.DEFAULT_TOLERANCE)
    rows = [_row(value=0.08)]
    failures = rtf.judge(rows, {"atmega328p": 1.0}, base, 0.10)
    assert len(failures) == 1 and "uno_feeder" in failures[0]
    assert rows[0]["index"] == pytest.approx(0.08)
    # Within tolerance, or no baseline / no spin figure: no failure.
    assert rtf.judge([_row(value=0.095)], {"atmega328p": 1.0}, base, 0.10) == []
    assert rtf.judge([_row(workload="new")], {"atmega328p": 1.0}, base, 0.10) == []
    assert rtf.judge([_row()], {}, base, 0.10) == []


def test_rtf_report_schema():
    row = {
        "board": "Arduino Uno", "chip": "atmega328p", "workload": "uno_feeder",
        "guest_s": 2.0, "wall_s": 1.0, "mcycles_per_s": 32.0, "rtf": 2.0,
    }
    doc = json.loads(json.dumps(rtf.report([row], [], 0.1)))
    for key in ("schema", "generated_at", "commit", "runner", "ok", "boards"):
        assert key in doc
    assert doc["ok"] is True
    for key in ("board", "workload", "guest_s", "wall_s", "rtf"):
        assert key in doc["boards"][0]


def test_rtf_baselines_name_known_workloads():
    known = {w.id for w in rtf.WORKLOADS}
    baselines = json.loads(rtf.BASELINES.read_text())
    assert set(baselines) <= known
    assert all(v["index"] > 0 for v in baselines.values())
