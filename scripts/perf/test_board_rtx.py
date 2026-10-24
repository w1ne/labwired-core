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


def test_declared_only_requires_no_platform_at_all(monkeypatch):
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: None)
    assert br.declared_only(DECLARED)
    assert not br.declared_only({**DECLARED, "peripherals": [{"id": "uart0"}]})
    assert not br.declared_only({**DECLARED, "flash": {"base": 0, "size": 1}})
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: "cortex-m")
    assert not br.declared_only(DECLARED)


def _run_main(monkeypatch, tmp_path, chips, waived, status):
    cli = tmp_path / "labwired"
    cli.write_text("")
    monkeypatch.setattr(br.bp, "discover_chips", lambda: chips)
    monkeypatch.setattr(br.bp, "plan_coverage", lambda c: ({}, waived))
    monkeypatch.setattr(br.bp, "build_fixtures", lambda needed: ({}, {}))
    monkeypatch.setattr(br.bp, "fixture_for", lambda chip: None)
    monkeypatch.setattr(
        "sys.argv", ["board_rtx.py", "--cli", str(cli), "--status-json", str(status)]
    )
    return br.main()


def test_a_waiver_on_a_runnable_chip_still_refuses(monkeypatch, tmp_path, capsys):
    runnable = {**DECLARED, "peripherals": [{"id": "uart0"}], "flash": {"base": 0, "size": 4096}}
    status = tmp_path / "status.json"
    code = _run_main(monkeypatch, tmp_path, {"a": runnable}, {"a": "because"}, status)
    assert code == 1
    assert "waivers remain on runnable chips: a" in capsys.readouterr().err
    assert not status.exists()


def test_a_declared_only_waiver_is_reported_not_refused(monkeypatch, tmp_path, capsys):
    status = tmp_path / "status.json"
    code = _run_main(monkeypatch, tmp_path, {"d": DECLARED}, {"d": "no platform"}, status)
    assert code == 0
    out = capsys.readouterr().out
    assert "not measured (declared only, no platform to run): d: no platform" in out
    assert "1 declared-only chip(s) not measured" in out
    import json
    assert json.loads(status.read_text())["not_measured"] == {"d": "no platform"}
