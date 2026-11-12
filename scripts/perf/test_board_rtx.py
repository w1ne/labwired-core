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


def test_rtf_gate_cancels_common_host_scaling_and_flags_a_slowdown():
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


def _pico_result(count=79):
    import hashlib
    return {
        "status": "pass", "stop_reason": "max_cycles", "cycles": 1_000_000_000,
        "inspect": {"devices": [{
            "id": "oled", "device_type": "oled-ssd1306", "declared": True,
            "attachment": {"transport": "i2c", "bus": "i2c0", "address": 0x3C},
            "artifacts": [{"id": "oled", "kind": "framebuffer", "meta": {
                "device_id": "oled", "format": "ssd1306_page", "w": 128, "h": 64,
                "powered": True, "frames_seen": [
                    {"t_us": 101_443 + i * 100_000, "ink_bytes": 24 + i,
                     "sha256": hashlib.sha256(f"counter {i + 1}".encode()).hexdigest()}
                    for i in range(count)
                ],
            }}],
        }]},
    }


def _pico_meta(result):
    return result["inspect"]["devices"][0]["artifacts"][0]["meta"]


PICO = next(w for w in rtf.WORKLOADS if w.id == "pico_oled_pwm")


def test_pico_progress_accepts_complete_counter_history_and_preserves_evidence():
    result = _pico_result()
    sample = rtf.parse_result(PICO, result, 2.0, 0)
    progress = sample["progress"]
    assert sample["cycles"] == 1_000_000_000 and sample["rtf"] == 4.0
    assert progress["unique_meaningful_frames"] == 79
    assert progress["first_t_us"] == 101_443 and progress["last_t_us"] == 7_901_443
    assert progress["frames"] == _pico_meta(result)["frames_seen"]
    assert progress["minimum_unique_frames"] == 75
    assert progress["minimum_last_t_us"] == 7_500_000


def test_pico_progress_accepts_exact_completion_boundary():
    result = _pico_result(75)
    for i, frame in enumerate(_pico_meta(result)["frames_seen"]):
        frame["t_us"] = (i + 1) * 100_000
    assert rtf.parse_result(PICO, result, 2.0, 0)["progress"]["last_t_us"] == 7_500_000


def test_pico_progress_refuses_old_rom_ready_then_wfi():
    # Same shape as the retained V1 run: READY and the complete cycle budget,
    # but only the first counter frame before analogWrite enters the ROM WFI.
    result = _pico_result(1)
    _pico_meta(result)["frames_seen"][0]["t_us"] = 101_531
    with pytest.raises(RuntimeError, match=r"need >= 75.*observed 1, last=101531"):
        rtf.parse_result(PICO, result, 2.0, 0)


@pytest.mark.parametrize("case", [
    "missing_inspect", "missing_devices", "duplicate_device", "wrong_type",
    "wrong_bus", "wrong_address", "undeclared", "duplicate_framebuffer",
    "wrong_artifact_id", "wrong_device_id", "wrong_dimensions", "float_dimensions", "wrong_format",
    "unpowered", "missing_history", "empty_history", "non_object_frame",
    "malformed_sha", "noninteger_time", "boolean_time", "reversed_time",
    "repeated_time", "past_budget", "invalid_ink", "boolean_ink", "blank_frames",
    "noise_frames", "repeated_picture", "insufficient_count", "ends_early",
])
def test_pico_progress_rejects_invalid_or_incomplete_evidence(case):
    result = _pico_result()
    panel = result["inspect"]["devices"][0]
    meta = _pico_meta(result)
    frames = meta["frames_seen"]
    if case == "missing_inspect": result.pop("inspect")
    elif case == "missing_devices": result["inspect"]["devices"] = None
    elif case == "duplicate_device": result["inspect"]["devices"].append(panel)
    elif case == "wrong_type": panel["device_type"] = "other"
    elif case == "wrong_bus": panel["attachment"]["bus"] = "i2c1"
    elif case == "wrong_address": panel["attachment"]["address"] = 0x3D
    elif case == "undeclared": panel["declared"] = False
    elif case == "duplicate_framebuffer": panel["artifacts"].append(panel["artifacts"][0])
    elif case == "wrong_artifact_id": panel["artifacts"][0]["id"] = "other"
    elif case == "wrong_device_id": meta["device_id"] = "other"
    elif case == "wrong_dimensions": meta["w"] = 64
    elif case == "float_dimensions": meta["w"] = 128.0
    elif case == "wrong_format": meta["format"] = "rgba"
    elif case == "unpowered": meta["powered"] = False
    elif case == "missing_history": meta.pop("frames_seen")
    elif case == "empty_history": meta["frames_seen"] = []
    elif case == "non_object_frame": frames[0] = None
    elif case == "malformed_sha": frames[0]["sha256"] = "not-a-frame-hash"
    elif case == "noninteger_time": frames[0]["t_us"] = "101443"
    elif case == "boolean_time": frames[0]["t_us"] = True
    elif case == "reversed_time": frames[1]["t_us"] = 100_000
    elif case == "repeated_time": frames[1]["t_us"] = frames[0]["t_us"]
    elif case == "past_budget": frames[-1]["t_us"] = 8_000_001
    elif case == "invalid_ink": frames[0]["ink_bytes"] = 1025
    elif case == "boolean_ink": frames[0]["ink_bytes"] = True
    elif case == "blank_frames":
        for frame in frames: frame["ink_bytes"] = 0
    elif case == "noise_frames":
        for frame in frames: frame["ink_bytes"] = 1
    elif case == "repeated_picture":
        for frame in frames: frame["sha256"] = frames[0]["sha256"]
    elif case == "insufficient_count": meta["frames_seen"] = frames[:74]
    elif case == "ends_early":
        for i, frame in enumerate(frames): frame["t_us"] = 100_000 + i * 90_000
    with pytest.raises(RuntimeError, match="OLED progress:"):
        rtf.parse_result(PICO, result, 2.0, 0)


@pytest.mark.parametrize("status,stop,code", [
    ("pass", "assertions_passed", 0), ("fail", "max_cycles", 1),
    ("pass", "max_steps", 0),
])
def test_pico_progress_does_not_replace_execution_completion(status, stop, code):
    result = _pico_result()
    result.update(status=status, stop_reason=stop)
    with pytest.raises(RuntimeError):
        rtf.parse_result(PICO, result, 2.0, code)


def test_rtf_retains_each_repeat_without_changing_the_median(monkeypatch):
    samples = [rtf.parse_result(PICO, _pico_result(), wall, 0) for wall in (2.0, 4.0, 1.0)]
    calls = iter(samples)
    monkeypatch.setattr(rtf, "run_once", lambda *args: next(calls))
    row = rtf.measure(Path("labwired"), PICO, 3)
    assert row["rtf"] == 4.0 and row["rtf_best"] == 8.0
    assert row["samples"] == samples
    assert [s["progress"]["unique_meaningful_frames"] for s in row["samples"]] == [79] * 3


def test_rtf_ratio_also_changes_when_only_spin_improves():
    rows = [_row(value=4.0)]
    # The workload remains 4x; a 40x -> 80x spin change halves its index.
    assert rtf.judge(rows, {"atmega328p": 80.0}, {"uno_feeder": {"index": 0.1}}, 0.25)
    assert rows[0]["index"] == 0.05
    assert rows[0]["spin_reference_rtx"] == 80.0


def test_rtf_provenance_records_actual_rom_override_and_input_hashes(monkeypatch, tmp_path):
    import hashlib
    cli = tmp_path / "labwired"
    cli.write_bytes(b"identified executable")
    rom = tmp_path / "selected-rom.bin"
    rom.write_bytes(b"explicit ROM override")
    monkeypatch.setenv("LABWIRED_RP2040_BOOTROM", str(rom))
    doc = rtf.measurement_provenance(cli, [PICO])
    assert doc["cli"]["sha256"] == hashlib.sha256(cli.read_bytes()).hexdigest()
    assert doc["rp2040_workload_rom"]["sha256"] == hashlib.sha256(rom.read_bytes()).hexdigest()
    assert doc["source_checkout"]["is_cli_build_proof"] is False
    for name in ("firmware", "system", "sketch", "chip"):
        entry = doc["inputs"][PICO.id][name]
        assert entry["sha256"] == hashlib.sha256(Path(entry["path"]).read_bytes()).hexdigest()
    assert set(doc["harness"]) == {"board_rtf.py", "board_rtx.py", "rtf_baselines.json"}


@pytest.mark.parametrize("workload_failure", [False, True])
def test_rtx_records_the_long_spin_samples_actually_used(monkeypatch, tmp_path, workload_failure):
    cli, fixture, status = tmp_path / "labwired", tmp_path / "spin.elf", tmp_path / "status.json"
    cli.write_bytes(b"cli")
    fixture.write_bytes(b"spin")
    (tmp_path / "rp2040.yaml").write_text("cpu_hz: 125000000\n")
    monkeypatch.setattr(br.bp, "CHIP_DIR", tmp_path)
    monkeypatch.setattr(br.bp, "discover_chips", lambda: {"rp2040": {"cpu_hz": 125_000_000}})
    monkeypatch.setattr(br.bp, "plan_coverage", lambda _: ({"rp2040": "rp2xxx"}, {}))
    monkeypatch.setattr(br.bp, "build_fixtures", lambda _: ({"rp2xxx": fixture}, {}))
    walls = iter((0.5, 0.4, 0.25, 1.0, 0.5, 0.75))
    monkeypatch.setattr(br, "run_once", lambda *args: (next(walls), 1024.0))
    rtf_out = tmp_path / "rtf.json"
    rtf_out.write_text('{"ok": true, "stale": true}')

    def workload_run(_cli, _chips, spin, _repeats, _tolerance, _out, **kwargs):
        assert spin == {"rp2040": 5.0}
        if workload_failure:
            raise RuntimeError("unexpected workload failure")
        return rtf.report([], [], 0.25, spin_reference=kwargs["spin_reference"])

    monkeypatch.setattr(br.board_rtf, "run", workload_run)
    monkeypatch.setattr("sys.argv", ["board_rtx.py", "--cli", str(cli),
                                     "--boards", "rp2040", "--status-json", str(status),
                                     "--rtf-json", str(rtf_out)])
    assert br.main() == (1 if workload_failure else 0)
    doc = json.loads(status.read_text())
    ref = doc["rtf"]["spin_reference"]["rp2040"]
    assert ref["steps"] == 312_500_000 and ref["best_rtx"] == 5.0
    assert ref["reference_seconds"] == 2.5
    assert [s["wall_s"] for s in ref["samples"]] == [1.0, 0.5, 0.75]
    assert [s["rtx"] for s in ref["samples"]] == [2.5, 5.0, 2.5 / 0.75]
    assert [s["wall_s"] for s in doc["measured"]["rp2040"]["samples"]] == [0.5, 0.4, 0.25]
    if workload_failure:
        failed = json.loads(rtf_out.read_text())
        assert failed == doc["rtf"] and failed["schema"] == 1
        assert failed["ok"] is False and "stale" not in failed


def test_failed_progress_overwrites_old_report_and_cannot_update_baselines(monkeypatch, tmp_path):
    out, baseline = tmp_path / "rtf.json", tmp_path / "baselines.json"
    out.write_text('{"ok": true, "stale": true}')
    original_baseline = '{"pico_oled_pwm": {"index": 0.0776}}\n'
    baseline.write_text(original_baseline)
    monkeypatch.setattr(rtf, "BASELINES", baseline)
    provenance = {"cli": {"sha256": "identified-cli"}, "source_checkout": {"head": "source-head"}}
    monkeypatch.setattr(rtf, "measurement_provenance", lambda *args: provenance)

    def stuck_workload(*args):
        return rtf.parse_result(PICO, _pico_result(1), 2.0, 0)

    monkeypatch.setattr(rtf, "measure", stuck_workload)
    spin = {"rp2040": {"best_rtx": 50.0, "samples": [{"wall_s": 1.0}]}}
    doc = rtf.run(Path("labwired"), {"rp2040"}, {"rp2040": 50.0}, 3, 0.25, out,
                  update=True, spin_reference=spin)
    assert doc["schema"] == 1 and doc["ok"] is False
    assert "OLED progress" in doc["failures"][0]
    assert doc["provenance"] == provenance and doc["spin_reference"] == spin
    assert json.loads(out.read_text()) == doc and "stale" not in doc
    assert baseline.read_text() == original_baseline
