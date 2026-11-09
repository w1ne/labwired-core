#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Realtime factor (RTF) of realistic per-board workloads.

``board_rtx.py`` proves every chip keeps up with its clock on a spin loop with
no attached parts. Users run sketches with displays, keypads and buses, where
peripheral servicing — not the CPU — sets the speed. This module runs one
committed stock-Arduino workload per main board through ``labwired test`` for
a fixed guest time and reports:

    guest_s = cycles / cpu_hz
    rtf     = guest_s / wall_s      (process startup included, like RTx)
    Mcycles/s

Wall clock depends on the runner, so the gate compares a host-independent
index instead: ``index = best workload RTF / best spin RTx`` of the same chip
in the same run.
A workload whose index drops more than ``--rtf-tolerance`` (default 25%)
below ``rtf_baselines.json`` fails. Workload sources, systems and ELFs live
in ``tests/fixtures/rtf/<id>/``.
"""

from __future__ import annotations

import json
import os
import platform
import statistics
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "tests" / "fixtures" / "rtf"
BASELINES = Path(__file__).resolve().parent / "rtf_baselines.json"
# Generous on purpose: wall clock on a shared runner moved the index by up to
# ~10% between back-to-back runs of the same build (one spin outlier moved it
# 50%). The deterministic Ir/step gate (board_perf.py) remains the sensitive
# detector; this one catches the big, user-visible slowdowns.
DEFAULT_TOLERANCE = 0.25


@dataclass(frozen=True)
class Workload:
    id: str
    board: str
    chip: str
    cpu_hz: int
    guest_s: float

    @property
    def dir(self) -> Path:
        return FIXTURES / self.id


WORKLOADS = (
    Workload("uno_feeder", "Arduino Uno", "atmega328p", 16_000_000, 2.0),
    Workload("pico_oled_pwm", "Raspberry Pi Pico", "rp2040", 125_000_000, 8.0),
    Workload("c3_i2c_uart", "ESP32-C3 DevKitM-1", "esp32c3", 160_000_000, 0.5),
    Workload("f411_timer_uart", "Black Pill F411", "stm32f411ceu6", 100_000_000, 0.5),
)


def rtf(cycles: int, cpu_hz: int, wall_s: float) -> float:
    if cpu_hz <= 0 or wall_s <= 0:
        raise ValueError("cpu_hz and wall_s must be positive")
    return (cycles / cpu_hz) / wall_s


def write_script(path: Path, w: Workload) -> None:
    cycles = round(w.cpu_hz * w.guest_s)
    path.write_text(
        "\n".join(
            [
                'schema_version: "1.0"',
                "inputs:",
                f'  firmware: "{w.dir / "firmware.elf"}"',
                f'  system: "{w.dir / "system.yaml"}"',
                "limits:",
                f"  max_steps: {cycles * 4}",
                f"  max_cycles: {cycles}",
                "  wall_time_ms: 600000",
                "  max_uart_bytes: 1000000",
                "assertions:",
                f'  - uart_contains: "RTF_READY {w.id}"',
                "",
            ]
        )
    )


def run_once(cli: Path, w: Workload) -> dict:
    """One timed run; raises unless the marker printed and the budget was spent."""
    with tempfile.TemporaryDirectory(prefix=f"rtf-{w.id}-") as tmp:
        tmp_path = Path(tmp)
        script = tmp_path / "test.yaml"
        write_script(script, w)
        env = dict(os.environ)
        env.setdefault(
            "LABWIRED_RP2040_BOOTROM",
            str(REPO_ROOT / "crates/core/roms/rp2040/bootrom.bin"),
        )
        start = time.perf_counter()
        proc = subprocess.run(
            [str(cli), "test", "--script", str(script), "--output-dir",
             str(tmp_path / "out"), "--no-uart-stdout"],
            capture_output=True, text=True, env=env, cwd=REPO_ROOT,
        )
        wall = time.perf_counter() - start
        try:
            result = json.loads((tmp_path / "out" / "result.json").read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise RuntimeError(f"{w.id}: no result.json ({exc}):\n{proc.stderr[-1500:]}")
    return parse_result(w, result, wall, proc.returncode)


def parse_result(w: Workload, result: dict, wall: float, returncode: int) -> dict:
    want = round(w.cpu_hz * w.guest_s)
    if returncode != 0 or result.get("status") != "pass":
        raise RuntimeError(
            f"{w.id}: run did not pass (exit {returncode}, status "
            f"{result.get('status')!r}, stop {result.get('stop_reason')!r})"
        )
    # `max_cycles` is checked against the machine clock, so a run that stopped
    # on it simulated exactly the budget. Any other stop would report an
    # inflated RTF for less guest time, so it is refused.
    if result.get("stop_reason") != "max_cycles":
        raise RuntimeError(f"{w.id}: stopped on {result.get('stop_reason')!r}, not max_cycles")
    return {"cycles": want, "wall_s": wall}


def measure(cli: Path, w: Workload, repeats: int) -> dict:
    runs = [run_once(cli, w) for _ in range(repeats)]
    rtfs = [rtf(r["cycles"], w.cpu_hz, r["wall_s"]) for r in runs]
    med = statistics.median(rtfs)
    pick = runs[rtfs.index(med)] if med in rtfs else runs[0]
    return {
        "board": w.board,
        "chip": w.chip,
        "workload": w.id,
        "cpu_hz": w.cpu_hz,
        "guest_s": pick["cycles"] / w.cpu_hz,
        "wall_s": round(pick["wall_s"], 4),
        "mcycles_per_s": round(pick["cycles"] / pick["wall_s"] / 1e6, 3),
        "rtf": round(med, 4),
        "rtf_min": round(min(rtfs), 4),
        "rtf_best": round(max(rtfs), 5),
        "repeats": repeats,
    }


def judge(rows: list[dict], spin_rtx: dict[str, float], baselines: dict,
          tolerance: float) -> list[str]:
    """Attach ``index``/``baseline_index`` to rows; return failure messages.

    ``index`` is the best workload RTF over the best spin RTx of the same chip
    in the same run. Host contention only ever slows a run down, so the best
    of N is the stable estimate; dividing by spin cancels the runner's speed.
    """
    failures = []
    for row in rows:
        spin = spin_rtx.get(row["chip"])
        if not spin:
            row["index"] = None
            continue
        row["index"] = float(f"{row.get('rtf_best', row['rtf']) / spin:.4g}")
        base = (baselines.get(row["workload"]) or {}).get("index")
        row["baseline_index"] = base
        if base is None:
            continue
        floor = base * (1.0 - tolerance)
        if row["index"] < floor:
            drop = 1.0 - row["index"] / base
            failures.append(
                f"{row['workload']} ({row['chip']}): index {row['index']:.4g} is "
                f"{drop:.0%} below baseline {base:.4g} (tolerance {tolerance:.0%})"
            )
    return failures


def git_commit() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, capture_output=True,
            text=True, check=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def report(rows: list[dict], failures: list[str], tolerance: float) -> dict:
    return {
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "commit": os.environ.get("GITHUB_SHA") or git_commit(),
        "runner": os.environ.get("RUNNER_NAME") or platform.node(),
        "host": {"machine": platform.machine(), "processor": platform.processor()},
        "tolerance": tolerance,
        "ok": not failures,
        "failures": failures,
        "boards": rows,
    }


def print_table(rows: list[dict]) -> None:
    print("\nworkload          chip            guest s   wall s   Mcyc/s   x real time   index")
    print("-" * 84)
    for r in rows:
        idx = "-" if r.get("index") is None else f"{r['index']:.4g}"
        print(
            f"{r['workload']:<17} {r['chip']:<15} {r['guest_s']:7.2f} {r['wall_s']:8.2f} "
            f"{r['mcycles_per_s']:8.2f} {r['rtf']:10.3f}x   {idx}"
        )


def run(cli: Path, chips: set[str], spin_rtx: dict[str, float], repeats: int,
        tolerance: float, out: Path | None, update: bool = False) -> dict | None:
    selected = [w for w in WORKLOADS if w.chip in chips]
    if not selected:
        return None
    rows = [measure(cli, w, repeats) for w in selected]
    baselines = json.loads(BASELINES.read_text()) if BASELINES.exists() else {}
    failures = judge(rows, spin_rtx, baselines, tolerance)
    if update:
        for r in rows:
            if r.get("index") is not None:
                baselines[r["workload"]] = {"index": r["index"]}
        BASELINES.write_text(json.dumps(baselines, indent=2, sort_keys=True) + "\n")
        failures = []
    print_table(rows)
    doc = report(rows, failures, tolerance)
    if out:
        out.write_text(json.dumps(doc, indent=2) + "\n")
    for f in failures:
        print(f"RTF regression: {f}")
    return doc
